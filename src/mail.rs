use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use mailparse::{MailHeaderMap, ParsedMail};
use native_tls::TlsConnector;
use regex::Regex;
use sha1::{Digest, Sha1};

use crate::store::{AccountSecret, NewMail};

pub fn extract_verification_code(content: &str) -> Option<String> {
    if content.trim().is_empty() {
        return None;
    }
    let html_block =
        Regex::new(r"background-color:\s*#F3F3F3[^>]*>[\s\S]*?(\d{6})[\s\S]*?</p>").ok()?;
    if let Some(hit) = html_block.captures(content).and_then(|cap| cap.get(1)) {
        return Some(hit.as_str().to_string());
    }
    for pattern in [r"Subject:.*?(\d{6})", r">\s*(\d{6})\s*<"] {
        if let Some(code) = first_non_placeholder_code(content, pattern, false) {
            return Some(code);
        }
    }
    first_non_placeholder_code(content, r"\b(\d{6})\b", true)
}

fn first_non_placeholder_code(
    content: &str,
    pattern: &str,
    skip_css_like_prefixes: bool,
) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    for hit in re.captures_iter(content).filter_map(|cap| cap.get(1)) {
        let code = hit.as_str();
        if code == "177010" {
            continue;
        }
        if skip_css_like_prefixes {
            let prefix = content[..hit.start()].chars().next_back();
            if matches!(prefix, Some('#' | '&')) {
                continue;
            }
        }
        return Some(code.to_string());
    }
    None
}

pub async fn fetch_latest_messages(account: AccountSecret, limit: usize) -> Result<Vec<NewMail>> {
    tokio::task::spawn_blocking(move || fetch_latest_messages_blocking(&account, limit))
        .await
        .context("imap worker panicked")?
}

fn fetch_latest_messages_blocking(account: &AccountSecret, limit: usize) -> Result<Vec<NewMail>> {
    let tls = TlsConnector::builder()
        .build()
        .context("build TLS connector")?;
    let client = imap::connect(
        (account.imap_host.as_str(), account.imap_port as u16),
        &account.imap_host,
        &tls,
    )
    .with_context(|| format!("connect IMAP {}", account.imap_host))?;
    let mut session = client
        .login(&account.email, &account.password)
        .map_err(|(err, _)| anyhow!("imap login failed: {err}"))?;
    let mailbox = session.select("INBOX").context("select INBOX")?;
    let exists = mailbox.exists;
    if exists == 0 {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    let safe_limit = limit.clamp(1, 50) as u32;
    let start = exists.saturating_sub(safe_limit).saturating_add(1);
    let sequence = format!("{start}:*");
    let fetches = session
        .fetch(sequence, "RFC822")
        .context("fetch latest messages")?;
    let mut out = Vec::new();
    for fetch in fetches.iter() {
        let Some(body) = fetch.body() else {
            continue;
        };
        if let Ok(parsed) = mailparse::parse_mail(body) {
            out.push(parsed_mail_to_new_mail(
                &account.email,
                fetch.message,
                &parsed,
                body,
            ));
        }
    }
    let _ = session.logout();
    out.sort_by_key(|mail| mail.received_at);
    out.reverse();
    Ok(out)
}

fn parsed_mail_to_new_mail(
    account_email: &str,
    sequence: u32,
    parsed: &ParsedMail<'_>,
    raw: &[u8],
) -> NewMail {
    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default();
    let from_addr = parsed.headers.get_first_value("From").unwrap_or_default();
    let received_at = parsed
        .headers
        .get_first_value("Date")
        .and_then(|value| DateTime::parse_from_rfc2822(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let (text, html) = collect_body_parts(parsed);
    let merged = format!("Subject: {subject}\n{text}\n{html}");
    let verification_code = extract_verification_code(&merged);
    let raw_hash = hash_raw(raw);

    NewMail {
        id: format!("{account_email}:{sequence}:{raw_hash}"),
        account_email: account_email.to_string(),
        subject,
        text,
        html,
        from_addr,
        received_at,
        verification_code,
        raw_hash,
    }
}

fn collect_body_parts(parsed: &ParsedMail<'_>) -> (String, String) {
    if parsed.subparts.is_empty() {
        let body = parsed.get_body().unwrap_or_default();
        let content_type = parsed.ctype.mimetype.to_ascii_lowercase();
        if content_type.contains("html") {
            return (String::new(), body);
        }
        return (body, String::new());
    }

    let mut text = Vec::new();
    let mut html = Vec::new();
    collect_body_parts_inner(parsed, &mut text, &mut html);
    (text.join("\n"), html.join("\n"))
}

fn collect_body_parts_inner(
    parsed: &ParsedMail<'_>,
    text: &mut Vec<String>,
    html: &mut Vec<String>,
) {
    if parsed.subparts.is_empty() {
        let body = parsed.get_body().unwrap_or_default();
        let content_type = parsed.ctype.mimetype.to_ascii_lowercase();
        if content_type.contains("html") {
            html.push(body);
        } else if content_type.contains("text") || !body.trim().is_empty() {
            text.push(body);
        }
        return;
    }
    for part in &parsed.subparts {
        collect_body_parts_inner(part, text, html);
    }
}

fn hash_raw(raw: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(raw);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::extract_verification_code;

    #[test]
    fn extracts_openai_six_digit_codes_and_ignores_known_false_positive() {
        assert_eq!(
            extract_verification_code("Subject: OpenAI code 123456\nYour code is 123456"),
            Some("123456".to_string())
        );
        assert_eq!(
            extract_verification_code("Ignore 177010 but keep 654321"),
            Some("654321".to_string())
        );
    }
}
