{
  const { chromium } = await import("playwright");
  const browser = await chromium.launch({ channel: "chrome", headless: true });
  try {
    const page = await browser.newPage();
    await page.goto("data:text/html,<title>sky-cua</title><h1>ready</h1>");
    nodeRepl.write({ title: await page.title(), text: await page.locator("h1").textContent() });
  } finally {
    await browser.close();
  }
}
