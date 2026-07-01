use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MailPayload {
    pub id: String,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub from: String,
    pub to: String,
    pub received_at: String,
    pub verification_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct YydsAccountData {
    pub id: String,
    pub address: String,
    pub token: String,
}

pub fn latest_response(mail: Option<MailPayload>) -> serde_json::Value {
    match mail {
        Some(mail) => serde_json::json!({
            "ok": true,
            "email": mail,
        }),
        None => serde_json::json!({
            "ok": false,
            "email": null,
        }),
    }
}

pub fn yyds_account_response(id: &str, address: &str, token: &str) -> serde_json::Value {
    serde_json::json!({
        "data": {
            "id": id,
            "address": address,
            "token": token,
        }
    })
}

pub fn yyds_messages_response(messages: Vec<MailPayload>) -> serde_json::Value {
    serde_json::json!({
        "data": messages,
    })
}

#[cfg(test)]
mod tests {
    use super::{MailPayload, latest_response, yyds_account_response, yyds_messages_response};

    fn sample_mail() -> MailPayload {
        MailPayload {
            id: "m1".to_string(),
            subject: "Your code is 123456".to_string(),
            text: "123456".to_string(),
            html: "<p>123456</p>".to_string(),
            from: "OpenAI <noreply@tm.openai.com>".to_string(),
            to: "user@outlook.com".to_string(),
            received_at: "2026-07-01T00:00:00Z".to_string(),
            verification_code: Some("123456".to_string()),
        }
    }

    #[test]
    fn latest_response_matches_gpt_register_self_hosted_contract() {
        let body = latest_response(Some(sample_mail()));

        assert_eq!(body["ok"], true);
        assert_eq!(body["email"]["subject"], "Your code is 123456");
        assert_eq!(body["email"]["verification_code"], "123456");
    }

    #[test]
    fn yyds_responses_match_gpt_register_provider_contract() {
        let account = yyds_account_response("a1", "user@outlook.com", "lease-token");
        assert_eq!(account["data"]["id"], "a1");
        assert_eq!(account["data"]["address"], "user@outlook.com");
        assert_eq!(account["data"]["token"], "lease-token");

        let messages = yyds_messages_response(vec![sample_mail()]);
        assert_eq!(messages["data"][0]["id"], "m1");
        assert_eq!(messages["data"][0]["verification_code"], "123456");
    }
}
