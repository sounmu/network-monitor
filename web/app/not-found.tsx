import Link from "next/link";

export default function NotFound() {
  return (
    <div className="glass-card" style={{ padding: "48px 24px", textAlign: "center" }}>
      <h1 style={{ fontSize: 24, marginBottom: 12 }}>Host Not Found</h1>
      <p style={{ color: "var(--text-muted)", marginBottom: 20 }}>
        The requested host does not exist or is no longer registered.
      </p>
      <Link
        href="/"
        style={{
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "10px 18px",
          borderRadius: 8,
          border: "1px solid var(--accent-blue)",
          background: "var(--accent-blue)",
          color: "var(--text-on-accent, #fff)",
          textDecoration: "none",
        }}
      >
        Back to Overview
      </Link>
    </div>
  );
}
