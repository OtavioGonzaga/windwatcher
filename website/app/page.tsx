import Link from "next/link";
import Image from "next/image";
import { publicUrl, repository } from "@/lib/site";

export default function Home() {
  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col justify-center gap-6 p-8">
      <Image
        src={publicUrl("/generated/docs/assets/windwatcher-banner.png")}
        alt="Windwatcher"
        width={960}
        height={320}
        priority
        className="h-auto w-full rounded-xl"
      />
      <h1 className="text-4xl font-bold">Windwatcher Documentation</h1>
      <p>
        Library-first messaging infrastructure for Rust and self-hosted
        deployments.
      </p>
      <Link href="/docs/" className="text-lg underline">
        Read the documentation →
      </Link>
      <div className="flex flex-wrap gap-6">
        <Link href="/docs/installation/" className="underline">
          Install the server
        </Link>
        <Link href="/docs/http-api/" className="underline">
          Explore the HTTP API
        </Link>
        <a href={repository} className="underline">
          GitHub
        </a>
      </div>
    </main>
  );
}
