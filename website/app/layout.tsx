import "./global.css";
import { Provider } from "@/components/provider";
import type { ReactNode } from "react";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: { default: "Windwatcher Documentation", template: "%s | Windwatcher" },
  description:
    "Guides, architectural decisions, and HTTP API reference for Windwatcher.",
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
