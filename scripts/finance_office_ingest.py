#!/usr/bin/python3
"""Validate admitted OOXML containers and produce a separate PDF derivative.
Run only in an OS-isolated worker with network denied, memory and CPU limits.
"""
import io
import os
import pathlib
import stat
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile

LIMIT = 25 * 1024 * 1024

def require(ok):
    if not ok:
        raise ValueError("invalid office input")

def validate(data, fmt):
    require(fmt in ("docx", "xlsx") and 0 < len(data) <= LIMIT)
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        entries = archive.infolist()
        require(0 < len(entries) <= 4096 and len({e.filename for e in entries}) == len(entries))
        require("[Content_Types].xml" in archive.namelist())
        require(("word/document.xml" if fmt == "docx" else "xl/workbook.xml") in archive.namelist())
        total = 0
        for e in entries:
            name = e.filename
            parts = pathlib.PurePosixPath(name).parts
            require(not name.startswith("/") and "\\" not in name and ":" not in name and ".." not in parts)
            require(not e.flag_bits & 1 and not stat.S_ISLNK(e.external_attr >> 16))
            lower = name.lower()
            require(not any(x in lower for x in ("vbaproject","activex","/embeddings/","externallinks","oleobject","customui","connections.xml","querytables/")))
            total += e.file_size
            require(total <= 100*1024*1024 and e.file_size <= LIMIT)
            if lower.endswith((".xml",".rels")):
                raw = archive.read(e)
                require(b"<!DOCTYPE" not in raw.upper() and b"<!ENTITY" not in raw.upper())
                root = ET.fromstring(raw)
                for node in root.iter():
                    require(node.attrib.get("TargetMode","").lower() != "external")
                    require("macroenabled" not in str(node.attrib).lower())
                    tag = node.tag.rsplit("}",1)[-1]
                    # Disable external fetch and execution mechanisms. Reject
                    # formulas rather than recalculate untrusted spreadsheet data.
                    require(tag not in ("f","instrText","fldSimple","object","altChunk"))

def main():
    require(len(sys.argv)==3)
    fmt, executable = sys.argv[1:]
    converter = pathlib.Path(executable)
    require(converter.is_absolute() and converter.is_file())
    data = sys.stdin.buffer.read(LIMIT+1)
    validate(data, fmt)
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="finance-ingest-") as directory:
        root = pathlib.Path(directory)
        source = root / ("source."+fmt)
        source.write_bytes(data)
        profile = root/"profile"
        profile.mkdir(mode=0o700)
        user = profile/"user"
        user.mkdir(mode=0o700)
        (user/"registrymodifications.xcu").write_text('<?xml version="1.0" encoding="UTF-8"?><oor:items xmlns:oor="http://openoffice.org/2001/registry"><item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item></oor:items>')
        subprocess.run([str(converter),"-env:UserInstallation="+profile.as_uri(),"--headless","--norestore","--nodefault","--convert-to","pdf","--outdir",directory,str(source)],check=True,timeout=90,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,env={"PATH":"/usr/bin:/bin","HOME":directory})
        output=root/"source.pdf"
        require(output.is_file() and 5<=output.stat().st_size<=LIMIT)
        result=output.read_bytes()
        require(result.startswith(b"%PDF-"))
        sys.stdout.buffer.write(result)

if __name__=="__main__":
    try:
        main()
    except (ValueError, OSError, zipfile.BadZipFile, ET.ParseError, subprocess.SubprocessError):
        sys.exit(65)
