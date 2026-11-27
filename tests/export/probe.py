"""Generate and verify actual synthetic office artifacts, no external data."""
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from docx import Document
from openpyxl import Workbook, load_workbook

MAX_ARTIFACT = 1024 * 1024


def command(args, cwd):
    result = subprocess.run(args, cwd=cwd, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=45, check=False)
    if result.returncode != 0 or len(result.stdout) > 65536:
        raise RuntimeError("office compatibility command failed")
    return result.stdout


def checked(path):
    if not path.is_file() or not 0 < path.stat().st_size <= MAX_ARTIFACT:
        raise ValueError("artifact size")
    return path.read_bytes()


def main():
    with tempfile.TemporaryDirectory(prefix="finance-export-", dir="/tmp") as d:
        root = Path(d)
        # Versioned synthetic fixture, NOT a company-approved form.
        template = root / "template.docx"
        doc = Document()
        doc.add_heading("SYNTHETIC PAYMENT REQUEST", 0)
        doc.add_paragraph("Dossier: {{dossier_code}}")
        doc.add_paragraph("Requested amount: {{amount}} {{currency}}")
        doc.add_paragraph("NOT FOR PAYMENT OR APPROVAL")
        doc.save(template)
        source_hash = hashlib.sha256(checked(template)).hexdigest()
        values = {"{{dossier_code}}":"SYN-EXPORT-001","{{amount}}":"125000","{{currency}}":"VND"}
        generated = Document(template)
        for paragraph in generated.paragraphs:
            for run in paragraph.runs:
                for marker, value in values.items():
                    run.text = run.text.replace(marker, value)
        assert not any("{{" in p.text for p in generated.paragraphs)
        docx = root / "request.docx"
        generated.save(docx)
        assert hashlib.sha256(checked(template)).hexdigest() == source_hash
        reread = Document(docx)
        assert any("125000 VND" in p.text for p in reread.paragraphs)

        xlsx = root / "checklist.xlsx"
        book = Workbook()
        sheet = book.active
        sheet.title = "Synthetic"
        sheet.append(["Field", "Value"])
        # Decimal string deliberately retained as text, not binary float.
        sheet.append(["Requested amount", "125000"])
        sheet.append(["Currency", "VND"])
        sheet.append(["Disposition", "UNVERIFIED"])
        book.save(xlsx)
        read = load_workbook(xlsx, read_only=True, data_only=False)
        assert read["Synthetic"]["B2"].value == "125000"
        assert read["Synthetic"]["B2"].data_type == "s"
        assert read["Synthetic"]["B4"].value == "UNVERIFIED"
        read.close()

        command(["libreoffice","-env:UserInstallation=file://"+str(root/"lo-profile"),
                 "--headless","--convert-to","pdf","--outdir",str(root),str(docx)],root)
        pdf = root / "request.pdf"
        assert checked(pdf).startswith(b"%PDF-")
        text = command(["pdftotext",str(pdf),"-"],root).decode("utf-8")
        assert "SYN-EXPORT-001" in text and "125000 VND" in text
        assert "NOT FOR PAYMENT OR APPROVAL" in text
        artifacts = []
        for path in [docx,xlsx,pdf]:
            payload=checked(path)
            artifacts.append({"name":path.name,"bytes":len(payload),
                              "sha256":hashlib.sha256(payload).hexdigest(),
                              "base64":base64.b64encode(payload).decode("ascii")})
        result={"schema":"finance.export.synthetic.v1","template_sha256":source_hash,
                "template_version":1,"artifacts":artifacts}
        raw=json.dumps(result,allow_nan=False).encode("utf-8")
        assert len(raw)<=4*1024*1024
        print(raw.decode("utf-8"))


if __name__=="__main__":
    main()
