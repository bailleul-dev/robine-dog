//! E-mail notifications: an address in a message (`@oncall@example.com`)
//! gets the alert, as Datadog's `@<email>` handles do, through the SMTP relay
//! of `ROBINE_DOG_SMTP_URL`.

use crate::AppState;
use lettre::message::{Mailbox, header::ContentType};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use regex::Regex;
use std::sync::{LazyLock, OnceLock};

/// `@name@domain.tld`, not preceded by a word character (so `a@b@c.d`'s
/// tail isn't one).
static HANDLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^\w@])@([A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,})")
        .unwrap()
});

/// The addresses a message notifies, in order, once each.
pub fn addresses(message: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in HANDLE.captures_iter(message) {
        let a = c[1].trim_end_matches('.').to_string();
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

static TRANSPORT: OnceLock<Option<AsyncSmtpTransport<Tokio1Executor>>> = OnceLock::new();

fn transport(st: &AppState) -> Option<&'static AsyncSmtpTransport<Tokio1Executor>> {
    TRANSPORT
        .get_or_init(|| {
            let url = st.cfg.smtp_url.as_deref()?;
            match AsyncSmtpTransport::<Tokio1Executor>::from_url(url) {
                Ok(b) => Some(b.build()),
                Err(e) => {
                    tracing::warn!(error = %e, "ROBINE_DOG_SMTP_URL is not usable");
                    None
                }
            }
        })
        .as_ref()
}

/// Sends `message` to each address it names; the handles notified.
pub async fn notify(st: &AppState, title: &str, message: &str) -> Vec<String> {
    let to = addresses(message);
    if to.is_empty() {
        return Vec::new();
    }
    let Some(smtp) = transport(st) else {
        tracing::warn!(
            addresses = ?to,
            "e-mail handles in a message but ROBINE_DOG_SMTP_URL is not set"
        );
        return Vec::new();
    };
    let from: Mailbox = match st.cfg.smtp_from.parse() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "ROBINE_DOG_SMTP_FROM is not an address");
            return Vec::new();
        }
    };
    let body = format!("{message}\n\n--\n{}/monitors", st.cfg.base_url);
    let mut sent = Vec::new();
    for addr in to {
        let Ok(rcpt) = addr.parse::<Mailbox>() else {
            continue;
        };
        let email = Message::builder()
            .from(from.clone())
            .to(rcpt)
            .subject(title)
            .header(ContentType::TEXT_PLAIN)
            .body(body.clone());
        let result = match email {
            Ok(m) => smtp.send(m).await.map(|_| ()).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(()) => sent.push(format!("@{addr}")),
            Err(e) => tracing::warn!(address = %addr, error = %e, "e-mail failed"),
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_addresses() {
        assert_eq!(
            addresses("CPU high @ops@example.com, cc @dev.team+x@mail.example.org. @webhook-x"),
            vec!["ops@example.com", "dev.team+x@mail.example.org"]
        );
        assert!(addresses("me@example.com wrote").is_empty());
        assert_eq!(addresses("@a@b.co @a@b.co"), vec!["a@b.co"]);
    }
}
