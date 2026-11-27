"""Real Chromium + HTTP + PostgreSQL synthetic E04 flow. No network provider."""
import os
from playwright.sync_api import sync_playwright, expect

origin = os.environ["FINANCE_TEST_ORIGIN"]
assert origin.startswith("http://127.0.0.1:")
token = os.environ.pop("FINANCE_TEST_TOKEN")
auditor = os.environ.pop("FINANCE_TEST_AUDITOR")
requester = os.environ.pop("FINANCE_TEST_REQUESTER")
with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    context = browser.new_context()
    page = context.new_page()
    page.set_default_timeout(15000)
    page.goto(origin)
    expect(page.get_by_role("heading", name="Danh sách đơn vị tổ chức")).to_be_visible()
    page.get_by_label("Token sandbox").fill(token)
    page.get_by_role("button", name="Kết nối", exact=True).click()
    expect(page.get_by_role("status")).to_contain_text("đơn vị từ API")
    assert page.locator("tbody tr").count() > 0
    expect(page.get_by_label("Token sandbox")).to_have_value("")
    page.get_by_role("button", name="Ngắt kết nối", exact=True).click()
    expect(page.get_by_role("status")).to_have_text("Đã ngắt kết nối.")
    assert page.locator("tbody tr").count() == 0
    page.get_by_label("Token sandbox").fill(auditor)
    page.get_by_role("button", name="Kết nối", exact=True).click()
    expect(page.get_by_role("status")).to_contain_text("(403)")
    assert page.locator("tbody tr").count() == 0
    page.get_by_role("button", name="E18 Quy trình").click()
    expect(page.get_by_role("heading", name="Hồ sơ tài chính nháp")).to_be_visible()
    page.get_by_label("Token sandbox").fill(requester)
    page.get_by_role("button", name="Kết nối", exact=True).click()
    draft_status = page.locator('p[role="status"][aria-live="polite"]')
    expect(draft_status).to_contain_text("hồ sơ từ API.")
    initial_dossiers = page.locator("tbody tr").count()
    assert initial_dossiers >= 3
    page.get_by_label("UUID đơn vị sandbox").fill("00000000-0000-0000-0000-00000000000a")
    page.get_by_label("Loại hồ sơ").select_option("SETTLEMENT")
    page.get_by_role("button", name="Tạo hồ sơ nháp").click()
    expect(page.get_by_role("heading", name="Chi tiết hồ sơ")).to_be_visible()
    expect(page.locator("tbody tr")).to_have_count(initial_dossiers + 1)
    expect(draft_status).to_contain_text("hồ sơ từ API.")
    docs = page.get_by_role("region",name="Metadata chứng từ")
    docs.get_by_label("File tổng hợp").set_input_files({"name":"browser-synthetic.pdf","mimeType":"application/pdf","buffer":b"%PDF-1.7 synthetic browser metadata"})
    expect(docs.locator(".document-status")).to_contain_text("Đã tính SHA-256")
    docs.get_by_role("button",name="Tạo metadata chứng từ").click()
    expect(docs.locator(".document-status")).to_have_text("Đã tải 1 chứng từ metadata.")
    expect(docs.get_by_text("PENDING_UPLOAD")).to_be_visible()
    page.get_by_role("button", name="Ngắt kết nối", exact=True).click()
    assert page.locator("tbody tr").count() == 0
    page.get_by_role("button", name="E19 Biểu mẫu").click()
    expect(page.get_by_role("heading", name="Form Designer")).to_be_visible()
    expect(page.get_by_label("Token PolicyAdmin sandbox")).to_have_value("")
    assert page.evaluate("Object.keys(localStorage).length + Object.keys(sessionStorage).length") == 0
    context.close()
    browser.close()
print("PASS: Chromium -> React -> actual JWT scoped HTTP -> PostgreSQL; unauthorized access and credential clearing")
