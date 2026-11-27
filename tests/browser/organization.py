"""Real Chromium + HTTP + PostgreSQL synthetic E04 flow. No network provider."""
import os
from playwright.sync_api import sync_playwright, expect

origin = os.environ["FINANCE_TEST_ORIGIN"]
assert origin.startswith("http://127.0.0.1:")
token = os.environ.pop("FINANCE_TEST_TOKEN")
auditor = os.environ.pop("FINANCE_TEST_AUDITOR")
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
    expect(page.get_by_role("heading", name="Chưa mở nghiệp vụ")).to_be_visible()
    assert page.get_by_label("Token sandbox").count() == 0
    assert page.evaluate("Object.keys(localStorage).length + Object.keys(sessionStorage).length") == 0
    context.close()
    browser.close()
print("PASS: Chromium -> React -> actual JWT scoped HTTP -> PostgreSQL; unauthorized access and credential clearing")
