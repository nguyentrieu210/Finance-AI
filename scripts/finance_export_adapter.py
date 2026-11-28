#!/usr/bin/python3
"""Fenced local export adapter. Requires a private artifact root and isolated service.
No database credentials, network requests, or untrusted command arguments.
"""
import argparse
import hashlib
import io
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
import tempfile
import uuid
import zipfile
import xml.etree.ElementTree as ET

LIMIT = 100 * 1024 * 1024
PLACEHOLDER = re.compile(r"\{\{([a-z][a-z0-9_.]{0,127})\}\}")
def require(ok):
    if not ok:
        raise ValueError("invalid render material or template")

def private(path):
    st = path.lstat()
    require(stat.S_ISDIR(st.st_mode) and st.st_mode & 0o077 == 0)

def put(path, data):
    require(0 < len(data) <= LIMIT)
    with path.open("xb") as out:
        out.write(data)
        out.flush()
        os.fsync(out.fileno())

def render_ooxml(source, values, fmt):
    result = io.BytesIO()
    total = 0
    with zipfile.ZipFile(io.BytesIO(source)) as zin, zipfile.ZipFile(result, "w", zipfile.ZIP_DEFLATED) as zout:
        entries = zin.infolist()
        require(len(entries) <= 4096 and len({x.filename for x in entries}) == len(entries))
        require("[Content_Types].xml" in zin.namelist())
        required = "word/document.xml" if fmt == "DOCX" else "xl/workbook.xml"
        require(required in zin.namelist())
        for entry in entries:
            name = entry.filename
            parts = pathlib.PurePosixPath(name).parts
            require(not name.startswith("/") and "\\" not in name and ".." not in parts and ":" not in name)
            lower = name.lower()
            require(not any(x in lower for x in ("vbaproject", "activex", "/embeddings/", "externallinks", "oleobject", "customui")))
            require(not entry.flag_bits & 1 and not stat.S_ISLNK(entry.external_attr >> 16))
            total += entry.file_size
            require(total <= LIMIT and entry.file_size <= 25*1024*1024)
            if entry.is_dir():
                continue
            data = zin.read(entry)
            if lower.endswith((".xml", ".rels")):
                require(b"<!DOCTYPE" not in data.upper() and b"<!ENTITY" not in data.upper())
                node = ET.fromstring(data)
                if lower.endswith(".rels"):
                    require(not any(e.attrib.get("TargetMode","").lower()=="external" for e in node.iter()))
                require(not any("macroenabled" in str(e.attrib).lower() for e in node.iter()))
                # Only complete placeholders in text nodes. Do not substitute code,
                # formulas, attributes, relationships or executable structures.
                for e in node.iter():
                    tag = e.tag.rsplit("}",1)[-1]
                    if e.text and ("{{" in e.text or "}}" in e.text):
                        require(tag in ("t","text"))
                        def replace(m):
                            key = m.group(1)
                            require(key in values)
                            return values[key]
                        e.text = PLACEHOLDER.sub(replace,e.text)
                        require("{{" not in e.text and "}}" not in e.text)
                data = ET.tostring(node,encoding="utf-8",xml_declaration=True)
            zout.writestr(name,data)
    return result.getvalue()

def main():
    p=argparse.ArgumentParser()
    for k in ("schema","job-id","tenant-id","company-id","fence"):
        p.add_argument("--"+k,required=True)
    a=p.parse_args()
    require(a.schema=="finance.export.request.v1" and 0<int(a.fence)<2**63)
    parts=[str(uuid.UUID(x)) for x in (a.tenant_id,a.company_id,a.job_id)]
    root=pathlib.Path(os.environ["FINANCE_EXPORT_ARTIFACT_ROOT"])
    require(root.is_absolute() and root.resolve()==root)
    private(root)
    directory=root
    for part in parts:
        directory=directory/part
        directory.mkdir(mode=0o700,exist_ok=True)
        private(directory)
    directory=directory/str(int(a.fence))
    directory.mkdir(mode=0o700)
    payload=json.loads(sys.stdin.buffer.read(70*1024*1024+1))
    m=payload["material"]
    require(m["schema"]=="finance.export.material.v1")
    source=bytes.fromhex(payload["template_hex"])
    require(0<len(source)<=25*1024*1024 and hashlib.sha256(source).hexdigest()==payload["template_sha256"]==m["template_digest"])
    values={}
    ambiguous=set()
    for field in m["fields"]:
        key,value=field["path"],field["value"]
        require(isinstance(value,str) and len(value)<=16384 and field["evidence"])
        if key in values:
            ambiguous.add(key)
        else:
            values[key]=value
    for key in ambiguous:
        values.pop(key,None)
    fmt=payload["format"]
    require(fmt in ("DOCX","XLSX","PDF"))
    name="request."+fmt.lower()
    if fmt=="PDF":
        # A static PDF is an attachment, not a fillable template.
        require(source.startswith(b"%PDF-") and not values)
        output=source
    else:
        output=render_ooxml(source,values,fmt)
    os.umask(0o077)
    put(directory/name,output)
    if fmt!="PDF":
        config=pathlib.Path(os.environ["FINANCE_EXPORT_ADAPTER_CONFIG"])
        require(config.is_absolute())
        settings=json.loads(config.read_text())
        converter=pathlib.Path(settings["libreoffice"])
        require(converter.is_absolute() and converter.is_file())
        with tempfile.TemporaryDirectory(prefix="finance-render-") as tmp:
            profile=pathlib.Path(tmp)/"profile"
            subprocess.run([str(converter),"-env:UserInstallation="+profile.as_uri(),"--headless","--convert-to","pdf","--outdir",tmp,str(directory/name)],check=True,timeout=90,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,env={"PATH":"/usr/bin:/bin","HOME":tmp})
            pdf=pathlib.Path(tmp)/"request.pdf"
            require(pdf.stat().st_size<=LIMIT)
            pdfbytes=pdf.read_bytes()
            require(pdfbytes.startswith(b"%PDF-"))
            put(directory/"request.pdf",pdfbytes)
    put(directory/"evidence-manifest.json",json.dumps(m,ensure_ascii=False,sort_keys=True,separators=(",",":")).encode())
    names=sorted(x.name for x in directory.iterdir())
    with zipfile.ZipFile(directory/"bundle.zip","x",zipfile.ZIP_DEFLATED) as z:
        for filename in names:
            z.write(directory/filename,filename)
    names.append("bundle.zip")
    artifacts=[]
    for filename in sorted(names):
        data=(directory/filename).read_bytes()
        require(0<len(data)<=LIMIT)
        digest=hashlib.sha256(data).hexdigest()
        artifacts.append({"name":filename,"expected_sha256":digest,"observed_sha256":digest,"bytes":len(data)})
    print(json.dumps({"schema":"finance.export.receipt.v1","job_id":parts[2],"fence":int(a.fence),"artifacts":artifacts},separators=(",",":")))

if __name__=="__main__":
    os.umask(0o077)
    try:
        main()
    except (ValueError,KeyError,TypeError,OSError,zipfile.BadZipFile,ET.ParseError,subprocess.SubprocessError):
        sys.exit(65)
