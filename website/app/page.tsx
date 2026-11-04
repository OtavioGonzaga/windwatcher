import Link from "next/link";
import Image from "next/image";
import { publicUrl, repository } from "@/lib/site";
import styles from "./home.module.css";

function Arrow({ diagonal = false }: { diagonal?: boolean }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {diagonal ? (
        <path d="M6 18 18 6M6 6h12v12" />
      ) : (
        <path d="M4 12h16m-6-6 6 6-6 6" />
      )}
    </svg>
  );
}

const guides = [
  {
    number: "01",
    label: "UNDERSTAND",
    title: "Start with the foundations",
    description:
      "Explore the library-first architecture, core concepts, and the decisions behind them.",
    href: "/docs/architecture/",
    action: "Explore the architecture",
  },
  {
    number: "02",
    label: "DEPLOY",
    title: "Make it your own",
    description:
      "Install the standalone server and learn how to configure, operate, and maintain it.",
    href: "/docs/installation/",
    action: "Install the server",
  },
  {
    number: "03",
    label: "INTEGRATE",
    title: "Connect your application",
    description:
      "Follow the HTTP guide, inspect the generated contract, and understand realtime delivery.",
    href: "/docs/http-api/",
    action: "Explore the HTTP API",
  },
];

export default function Home() {
  return (
    <div className={styles.home}>
      <header className={styles.header}>
        <Link href="/" aria-label="Windwatcher home" className={styles.brand}>
          <span className={styles.logo}>
            <Image
              src={publicUrl("/generated/docs/assets/windwatcher-banner.png")}
              alt="Windwatcher"
              width={540}
              height={180}
              priority
            />
          </span>
          <span className={styles.brandLabel}>Documentation</span>
        </Link>
        <nav aria-label="Main navigation" className={styles.nav}>
          <a href={repository} className={styles.github}>
            GitHub <Arrow diagonal />
          </a>
          <Link href="/docs/" className={styles.navButton}>
            Browse docs <Arrow />
          </Link>
        </nav>
      </header>

      <main className={styles.main}>
        <section className={styles.hero} aria-labelledby="hero-title">
          <div className={styles.heroCopy}>
            <p className={styles.eyebrow}>
              <span /> LIBRARY-FIRST. SELF-HOSTED.
            </p>
            <h1 id="hero-title">
              Messaging,
              <br />
              <span>on your terms.</span>
            </h1>
            <p className={styles.lead}>
              A reusable Rust core. A server you control.
              <br className={styles.desktopBreak} /> Build messaging into your
              product, without giving up your infrastructure.
            </p>
            <div className={styles.actions}>
              <Link href="/docs/" className={styles.primaryButton}>
                Read the documentation <Arrow />
              </Link>
              <Link href="/docs/http-api/" className={styles.secondaryButton}>
                HTTP API guide <Arrow diagonal />
              </Link>
            </div>
            <ul className={styles.traits} aria-label="Project foundations">
              <li>Written in Rust</li>
              <li>HTTP + WebSocket</li>
              <li>OpenAPI 3.1</li>
            </ul>
          </div>

          <figure className={styles.blueprint}>
            <div className={styles.blueprintBar}>
              <span className={styles.windowDots} aria-hidden="true">
                <i />
                <i />
                <i />
              </span>
              <span>DESIGNED TO FIT YOUR STACK</span>
            </div>
            <div className={styles.blueprintBody}>
              <div className={styles.hosts}>
                <div className={styles.host}>
                  <span className={styles.nodeIcon} aria-hidden="true">
                    {"{ }"}
                  </span>
                  <strong>Your application</strong>
                  <span>
                    Your runtime.
                    <br />
                    Your transport.
                  </span>
                </div>
                <div className={styles.host}>
                  <span className={styles.nodeIcon} aria-hidden="true">
                    {">_"}
                  </span>
                  <strong>Windwatcher server</strong>
                  <span>
                    HTTP API.
                    <br />
                    WebSocket realtime.
                  </span>
                </div>
              </div>
              <div className={styles.connections} aria-hidden="true">
                <span />
                <span />
              </div>
              <div className={styles.core}>
                <span className={styles.coreLabel}>THE SHARED FOUNDATION</span>
                <strong>
                  windwatcher<span>.rs</span>
                </strong>
                <span>Reusable, embeddable messaging library</span>
              </div>
              <div className={styles.adapterLine} aria-hidden="true" />
              <div className={styles.adapters}>
                <span>SQLite</span>
                <span>PostgreSQL</span>
                <span>MySQL</span>
                <span>MongoDB</span>
              </div>
            </div>
            <figcaption>
              <span className={styles.captionDot} />
              One core. Two ways to build.
            </figcaption>
          </figure>
        </section>

        <section className={styles.guides} aria-labelledby="guides-title">
          <div className={styles.sectionHeading}>
            <div>
              <p className={styles.eyebrow}>YOUR NEXT STEP</p>
              <h2 id="guides-title">Find your starting point.</h2>
            </div>
            <p>
              From the first concept
              <br />
              to your own deployment.
            </p>
          </div>
          <div className={styles.guideGrid}>
            {guides.map((guide) => (
              <Link
                href={guide.href}
                className={styles.guideCard}
                key={guide.number}
              >
                <div className={styles.cardMeta}>
                  <span>{guide.label}</span>
                  <span>{guide.number}</span>
                </div>
                <h3>{guide.title}</h3>
                <p>{guide.description}</p>
                <span className={styles.cardAction}>
                  {guide.action}
                  <Arrow />
                </span>
              </Link>
            ))}
          </div>
        </section>

        <section className={styles.reference} aria-labelledby="reference-title">
          <div className={styles.referenceMark} aria-hidden="true">
            {"</>"}
          </div>
          <div>
            <h2 id="reference-title">
              The implementation is the source of truth.
            </h2>
            <p>
              Guides from the repository. HTTP reference from the Rust contract.
              No second specification to keep in sync.
            </p>
          </div>
          <Link href="/docs/api/sessions/createSession/">
            Browse the API reference <Arrow diagonal />
          </Link>
        </section>
      </main>

      <footer className={styles.footer}>
        <p>
          Windwatcher <span>·</span> Open source. On your infrastructure.
        </p>
        <nav aria-label="Resources">
          <a href={repository}>
            GitHub <Arrow diagonal />
          </a>
          <a href="https://docs.rs/windwatcher">
            Rust API <Arrow diagonal />
          </a>
          <Link href="/docs/adr/">Design decisions</Link>
          <a href={publicUrl("/llms.txt")}>For agents</a>
        </nav>
      </footer>
    </div>
  );
}
