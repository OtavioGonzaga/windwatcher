import { test, expect } from "@playwright/test";
import { basePath } from "../../lib/site";

test("static pages, assets and nested refresh work without runtime errors", async ({
  page,
  request,
}, info) => {
  const failures: string[] = [];
  page.on("pageerror", (error) => failures.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") failures.push(message.text());
  });
  page.on("response", (response) => {
    if (response.status() >= 400)
      failures.push(`${response.status()} ${response.url()}`);
  });
  page.on("requestfailed", (request) => {
    if (request.failure()?.errorText !== "net::ERR_ABORTED")
      failures.push(`${request.url()}: ${request.failure()?.errorText}`);
  });
  for (const route of [
    "",
    "docs/",
    "docs/architecture/",
    "docs/installation/",
    "docs/http-api/",
    "docs/api/sessions/createSession/",
    "docs/api/messages/sendMessage/",
    "docs/api/realtime/connectRealtime/",
    "docs/realtime/",
    "docs/adr/",
    "docs/adr/0010-http-v1-transport-and-contracts/",
  ]) {
    await page.goto(`${basePath}/${route}`);
    await expect(page.locator("h1")).toHaveCount(1);
    await expect(page.locator("h1")).not.toHaveText("404");
    await expect(page.locator("body")).not.toContainText("Application error");
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth + 1,
    );
    expect(overflow, `Horizontal overflow: ${route}`).toBe(false);
    if (route === "docs/" || route === "docs/api/messages/sendMessage/") {
      const screenshot = info.outputPath(
        route === "docs/" ? "documentation.png" : "api-reference.png",
      );
      await page.screenshot({ path: screenshot, fullPage: true });
      await info.attach(route === "docs/" ? "Documentation" : "API reference", {
        path: screenshot,
        contentType: "image/png",
      });
    }
  }
  await page.reload();
  await expect(page.locator("h1")).toContainText("0010");
  const icon = page.locator('link[rel="icon"]');
  await expect(icon).toHaveAttribute("href");
  const iconUrl = new URL((await icon.getAttribute("href"))!, page.url());
  expect(iconUrl.pathname).toBe(`${basePath}/favicon.ico`);
  const favicon = await request.get(iconUrl.href);
  expect(favicon.ok()).toBeTruthy();
  expect(favicon.headers()["content-type"]).toContain("image/");
  const spec = await request.get(`${basePath}/openapi/v1.json`);
  expect(spec.ok()).toBeTruthy();
  expect((await spec.json()).openapi).toBe("3.1.0");
  for (const file of [
    "llms.txt",
    "llms-full.txt",
    "markdown/docs/architecture/content.md",
    "markdown/docs/api/messages/sendMessage/content.md",
  ]) {
    const artifact = await request.get(`${basePath}/${file}`);
    expect(artifact.ok()).toBeTruthy();
    expect(await artifact.text()).not.toContain("<!DOCTYPE html>");
  }
  expect(failures).toEqual([]);
});

test("client navigation and search cover guides, ADRs and HTTP operations", async ({
  page,
}) => {
  await page.goto(`${basePath}/`);
  await page.getByRole("link", { name: "Read the documentation" }).click();
  await expect(page).toHaveURL(new RegExp(`${basePath}/docs/$`));
  for (const [query, expected] of [
    ["Architecture", "/docs/architecture/"],
    ["0010", "/docs/adr/0010-http-v1-transport-and-contracts/"],
    ["sendMessage", "/docs/api/messages/sendMessage/"],
  ]) {
    await page
      .getByRole("button", { name: /search/i })
      .first()
      .click();
    const input = page.getByRole("dialog").getByRole("combobox");
    await input.fill(query);
    const result = page
      .getByRole("dialog")
      .getByRole("option", {
        name:
          query === "Architecture"
            ? /^Architecture$/
            : query === "0010"
              ? /^0010: HTTP v1 transport and contracts$/
              : /sendMessage$/,
      })
      .first();
    await expect(result).toBeVisible();
    await result.click();
    await expect(page).toHaveURL(new RegExp(`${basePath}${expected}(#.*)?$`));
  }
});

test("home presents clear entry points in light and dark themes", async ({
  page,
}, info) => {
  await page.goto(`${basePath}/`);
  await expect(page.getByRole("heading", { level: 1 })).toContainText(
    "on your terms.",
  );
  for (const title of [
    "Start with the foundations",
    "Make it your own",
    "Connect your application",
  ]) {
    await expect(
      page.getByRole("link", { name: new RegExp(title) }),
    ).toBeVisible();
  }
  await expect(page.getByRole("link", { name: "Browse docs" })).toBeVisible();
  for (const theme of ["light", "dark"]) {
    await page.evaluate((theme) => localStorage.setItem("theme", theme), theme);
    await page.reload();
    await expect(page.locator("html")).toHaveClass(new RegExp(theme));
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth + 1,
      ),
    ).toBe(true);
    const screenshot = info.outputPath(`home-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await info.attach(`Home (${theme})`, {
      path: screenshot,
      contentType: "image/png",
    });
  }
});

test("theme switching persists and mobile navigation is usable", async ({
  page,
}, info) => {
  await page.goto(`${basePath}/docs/architecture/`);
  if (info.project.name === "mobile") {
    await page.getByRole("button", { name: /open sidebar/i }).click();
    await expect(
      page.getByRole("link", {
        name: "Linux installation and system services",
      }),
    ).toBeVisible();
  }
  const toggle = page.getByRole("button", { name: /toggle theme/i });
  if (await toggle.count()) {
    const before = await page.locator("html").getAttribute("class");
    await toggle.first().click();
    await expect(page.locator("html")).not.toHaveAttribute(
      "class",
      before ?? "",
    );
  } else {
    const dark = page.getByRole("radio", { name: /dark/i });
    await dark.click();
    await expect(page.locator("html")).toHaveClass(/dark/);
  }
  const selected = await page.locator("html").getAttribute("class");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("class", selected ?? "");
});
