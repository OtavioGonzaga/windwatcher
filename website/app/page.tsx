import Link from "next/link";

export default function Home() {
  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col justify-center gap-6 p-8">
      <h1 className="text-4xl font-bold">Windwatcher</h1>
      <p>
        Library-first messaging infrastructure for Rust and self-hosted
        deployments.
      </p>
      <Link href="/docs/" className="text-lg underline">
        Read the documentation →
      </Link>
    </main>
  );
}
