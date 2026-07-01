#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAccount {
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    pub line_number: usize,
    pub line: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImport {
    pub accounts: Vec<ImportedAccount>,
    pub errors: Vec<ImportError>,
}

const DEFAULT_IMAP_HOST: &str = "outlook.office365.com";
const DEFAULT_IMAP_PORT: u16 = 993;

pub fn parse_import_text(input: &str) -> ParsedImport {
    let mut accounts = Vec::new();
    let mut errors = Vec::new();

    for (index, raw_line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        match parse_line(line) {
            Ok(account) => accounts.push(account),
            Err(reason) => errors.push(ImportError {
                line_number,
                line: line.to_string(),
                reason,
            }),
        }
    }

    ParsedImport { accounts, errors }
}

fn parse_line(line: &str) -> Result<ImportedAccount, String> {
    let parts = split_line(line)?;
    let email = parts
        .first()
        .map_or("", String::as_str)
        .trim()
        .to_lowercase();
    let password = parts.get(1).map_or("", String::as_str).trim().to_string();
    let imap_host = parts
        .get(2)
        .map_or(DEFAULT_IMAP_HOST, String::as_str)
        .trim()
        .to_string();
    let imap_port = parts.get(3).map_or(Ok(DEFAULT_IMAP_PORT), |raw| {
        raw.trim()
            .parse::<u16>()
            .map_err(|_| "imap_port must be a number".to_string())
    })?;

    if !looks_like_email(&email) {
        return Err("email is invalid".to_string());
    }
    if password.is_empty() {
        return Err("password is required".to_string());
    }
    if imap_host.is_empty() {
        return Err("imap_host is required".to_string());
    }

    Ok(ImportedAccount {
        email,
        password,
        imap_host,
        imap_port,
    })
}

fn split_line(line: &str) -> Result<Vec<String>, String> {
    let delimiter = if line.contains(',') { ',' } else { ':' };
    let parts: Vec<String> = line
        .split(delimiter)
        .map(|part| part.trim().trim_matches('"').to_string())
        .collect();
    if parts.len() < 2 {
        return Err("password is required".to_string());
    }
    Ok(parts)
}

fn looks_like_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.trim().is_empty() && domain.contains('.') && !domain.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::parse_import_text;

    #[test]
    fn parses_csv_and_colon_outlook_accounts_with_defaults() {
        let parsed = parse_import_text(
            "alpha@outlook.com,alpha-pass\n\
             beta@hotmail.com:beta-pass\n\
             gamma@outlook.com,gamma-pass,imap-mail.outlook.com,993\n",
        );

        assert_eq!(parsed.errors, Vec::new());
        assert_eq!(parsed.accounts.len(), 3);
        assert_eq!(parsed.accounts[0].email, "alpha@outlook.com");
        assert_eq!(parsed.accounts[0].password, "alpha-pass");
        assert_eq!(parsed.accounts[0].imap_host, "outlook.office365.com");
        assert_eq!(parsed.accounts[0].imap_port, 993);
        assert_eq!(parsed.accounts[1].email, "beta@hotmail.com");
        assert_eq!(parsed.accounts[1].password, "beta-pass");
        assert_eq!(parsed.accounts[2].imap_host, "imap-mail.outlook.com");
    }

    #[test]
    fn reports_invalid_lines_without_dropping_valid_accounts() {
        let parsed = parse_import_text(
            "not-an-email,password\n\
             ok@outlook.com,secret\n\
             missing-password@outlook.com\n",
        );

        assert_eq!(parsed.accounts.len(), 1);
        assert_eq!(parsed.accounts[0].email, "ok@outlook.com");
        assert_eq!(parsed.errors.len(), 2);
        assert_eq!(parsed.errors[0].line_number, 1);
        assert!(parsed.errors[0].reason.contains("email"));
        assert_eq!(parsed.errors[1].line_number, 3);
        assert!(parsed.errors[1].reason.contains("password"));
    }
}
