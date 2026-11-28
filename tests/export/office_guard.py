"""Bounded OOXML inspection for generated synthetic template/artifact compatibility."""
from io import BytesIO
from pathlib import PurePosixPath
import posixpath
import stat
import xml.etree.ElementTree as ET
import zipfile

MAX_ARCHIVE=25*1024*1024
MAX_EXPANDED=64*1024*1024
MAX_ENTRY=16*1024*1024
BLOCKED=("vbaproject","vbadata","activex/","embeddings/","oleobject","customui/","externallinks/","connections.xml")
REL="{http://schemas.openxmlformats.org/package/2006/relationships}Relationship"


def inspect(payload,kind):
    if kind not in {"docx","xlsx"} or not 0<len(payload)<=MAX_ARCHIVE:
        raise ValueError("unsupported archive")
    with zipfile.ZipFile(BytesIO(payload)) as z:
        infos=z.infolist()
        if not 0<len(infos)<=512:
            raise ValueError("entry count")
        names=set()
        expanded=0
        for info in infos:
            name=info.filename
            parts=PurePosixPath(name).parts
            if (not name or name.startswith("/") or "\\" in name or ":" in name
                or any(p in {".",".."} for p in name.split("/")) or not parts
                or name.casefold() in names or info.flag_bits&1
                or stat.S_ISLNK((info.external_attr>>16)&0xffff)
                or any(token in name.casefold() for token in BLOCKED)):
                raise ValueError("unsafe archive entry")
            names.add(name.casefold())
            expanded+=info.file_size
            if info.file_size>MAX_ENTRY or expanded>MAX_EXPANDED or (info.file_size>1024*1024 and info.file_size>max(1,info.compress_size)*100):
                raise ValueError("expansion limit")
        required={"[content_types].xml","_rels/.rels","word/document.xml" if kind=="docx" else "xl/workbook.xml"}
        if not required<=names:
            raise ValueError("wrong package type")
        for info in infos:
            if not info.filename.lower().endswith((".xml",".rels")):
                continue
            with z.open(info) as stream:
                raw=stream.read(MAX_ENTRY+1)
            if len(raw)!=info.file_size or len(raw)>MAX_ENTRY:
                raise ValueError("entry size mismatch")
            # UTF-8 only in this narrow probe; avoid alternate encodings hiding DTD.
            text=raw.decode("utf-8-sig")
            upper=text.upper()
            if "<!DOCTYPE" in upper or "<!ENTITY" in upper:
                raise ValueError("DTD or entity")
            root=ET.fromstring(text)
            if info.filename.lower()=="[content_types].xml":
                for child in root:
                    content=child.attrib.get("ContentType","").lower()
                    if any(token in content for token in ("macroenabled","vba","activex","oleobject")):
                        raise ValueError("active content type")
            if info.filename.lower().endswith(".rels"):
                parent=posixpath.dirname(info.filename)
                base=posixpath.dirname(parent) if parent.endswith("_rels") else ""
                for rel in root.iter(REL):
                    target=rel.attrib.get("Target","")
                    if rel.attrib.get("TargetMode","Internal")!="Internal" or not target or ":" in target or "\\" in target or "%" in target:
                        raise ValueError("external or invalid relationship")
                    resolved=posixpath.normpath(target.lstrip("/") if target.startswith("/") else posixpath.join(base,target))
                    if resolved==".." or resolved.startswith("../") or resolved.casefold() not in names:
                        raise ValueError("unresolved relationship")


def negative_probes(valid):
    import zipfile
    def altered(name,body):
        out=BytesIO()
        with zipfile.ZipFile(BytesIO(valid)) as original,zipfile.ZipFile(out,"w",zipfile.ZIP_DEFLATED) as dest:
            for info in original.infolist():
                if info.filename!=name:
                    dest.writestr(info.filename,original.read(info))
            dest.writestr(name,body)
        return out.getvalue()
    for candidate in (
        altered("../evil.xml",b"<x/>"),
        altered("word/vbaProject.bin",b"synthetic macro marker"),
        altered("_rels/.rels",b'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" TargetMode="External" Target="https://example.invalid/x"/></Relationships>'),
        altered("word/document.xml",b'<!DOCTYPE x [<!ENTITY a "x">]><x>&a;</x>'),
    ):
        try:
            inspect(candidate,"docx")
        except (ValueError,ET.ParseError,zipfile.BadZipFile):
            pass
        else:
            raise AssertionError("unsafe synthetic template accepted")
