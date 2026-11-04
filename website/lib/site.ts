// Empty for a custom domain; GitHub project Pages uses /windwatcher.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? "/windwatcher";
if (basePath !== "" && (!basePath.startsWith("/") || basePath.endsWith("/"))) {
  throw new Error(
    "NEXT_PUBLIC_BASE_PATH must be empty or start (but not end) with /",
  );
}

export const repository = "https://github.com/OtavioGonzaga/windwatcher";
export const siteOrigin =
  process.env.NEXT_PUBLIC_SITE_ORIGIN ?? "https://otaviogonzaga.github.io";

/** For raw assets and fetch URLs. Next Link already applies basePath. */
export function publicUrl(path: string) {
  return `${basePath}${path}`;
}
