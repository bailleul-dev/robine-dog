import { Logo } from "../components/Icons";

export function Login() {
  const params = new URLSearchParams(window.location.search);
  const error = params.get("error");
  const signedOut = params.has("signed_out");
  const path = window.location.pathname;
  const returnTo = path === "/login" || path === "/" ? "/logs" : path + window.location.search;
  return (
    <div className="login">
      <div className="login-card">
        <Logo className="logo" />
        <h1>robine-dog</h1>
        <p>Logs & traces, for one.</p>
        {error && <div className="login-error">{error}</div>}
        {signedOut && !error && <div className="muted" style={{ marginBottom: 16 }}>You are signed out.</div>}
        <a className="btn primary" href={`/auth/login?return_to=${encodeURIComponent(returnTo)}`}>
          Sign in with Robine ID
        </a>
      </div>
    </div>
  );
}
