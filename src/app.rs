use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::warn;

use crate::{
    auth::{AuthError, authorize_header},
    compat::{MailPayload, latest_response, yyds_account_response, yyds_messages_response},
    config::AppConfig,
    importer::{ImportError, parse_import_text},
    mail::fetch_latest_messages,
    store::{AccountSecret, Store, StoredMail},
};

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: AppConfig,
    pub imap_permits: Arc<Semaphore>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/api/latest", get(api_latest))
        .route("/v1/accounts", post(yyds_create_account))
        .route("/v1/messages", get(yyds_messages))
        .route("/v1/messages/{id}", get(yyds_message_detail))
        .route("/api/admin/import", post(admin_import))
        .route("/api/admin/accounts", get(admin_accounts))
        .route("/api/admin/accounts/{id}/test", post(admin_test_account))
        .route("/api/admin/accounts/{id}", delete(admin_delete_account))
        .route("/api/admin/stats", get(admin_stats))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

#[derive(Debug, Deserialize)]
struct LatestQuery {
    address: String,
}

async fn api_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LatestQuery>,
) -> Result<Json<Value>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    if let Some(account) = state.store.get_account_by_email(&query.address).await? {
        let _ = refresh_account(&state, account).await;
    }
    let mail = state.store.latest_mail_for(&query.address).await?;
    Ok(Json(latest_response(mail.map(mail_payload))))
}

#[derive(Debug, Deserialize)]
struct YydsCreateAccountRequest {
    address: Option<String>,
    domain: Option<String>,
}

async fn yyds_create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<YydsCreateAccountRequest>,
) -> Result<Json<Value>, AppError> {
    authorize_x_api_key(&headers, &state.config.api_key)?;
    let _requested_address = payload.address.as_deref().unwrap_or_default();
    let _requested_domain = payload.domain.as_deref().unwrap_or_default();
    let Some(lease) = state
        .store
        .lease_account(state.config.lease_ttl_seconds)
        .await?
    else {
        return Err(AppError::status(
            StatusCode::CONFLICT,
            "no idle accounts available",
        ));
    };
    Ok(Json(yyds_account_response(
        &lease.id,
        &lease.email,
        &lease.token,
    )))
}

async fn yyds_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| AppError::status(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    if let Some(account) = state.store.get_account_by_token(&token).await? {
        refresh_account(&state, account).await?;
    }
    let messages = state.store.latest_messages_for_token(&token).await?;
    Ok(Json(yyds_messages_response(
        messages.into_iter().map(mail_payload).collect(),
    )))
}

async fn yyds_message_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| AppError::status(StatusCode::UNAUTHORIZED, "missing bearer token"))?;
    let Some(message) = state.store.message_for_token(&token, &id).await? else {
        return Err(AppError::status(StatusCode::NOT_FOUND, "message not found"));
    };
    Ok(Json(json!({"data": mail_payload(message)})))
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    text: String,
}

#[derive(Debug, Serialize)]
struct ImportResponse {
    imported: usize,
    accepted: usize,
    errors: Vec<ImportErrorDto>,
}

#[derive(Debug, Serialize)]
struct ImportErrorDto {
    line_number: usize,
    line: String,
    reason: String,
}

async fn admin_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ImportRequest>,
) -> Result<Json<ImportResponse>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    let parsed = parse_import_text(&payload.text);
    let imported = state.store.import_accounts(&parsed.accounts).await?;
    Ok(Json(ImportResponse {
        imported,
        accepted: parsed.accounts.len(),
        errors: parsed.errors.into_iter().map(import_error_dto).collect(),
    }))
}

async fn admin_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    let accounts = state.store.list_accounts().await?;
    Ok(Json(json!({ "accounts": accounts })))
}

async fn admin_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    Ok(Json(json!({ "stats": state.store.stats().await? })))
}

async fn admin_test_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    let Some(account) = state.store.get_account_secret(&id).await? else {
        return Err(AppError::status(StatusCode::NOT_FOUND, "account not found"));
    };
    refresh_account(&state, account).await?;
    Ok(Json(json!({"ok": true})))
}

async fn admin_delete_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize(&headers, &state.config.api_key)?;
    let deleted = state.store.delete_account(&id).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn refresh_account(state: &AppState, account: AccountSecret) -> Result<(), AppError> {
    let _permit = state
        .imap_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            AppError::status(StatusCode::SERVICE_UNAVAILABLE, "imap worker unavailable")
        })?;
    match fetch_latest_messages(account.clone(), 20).await {
        Ok(messages) => {
            state.store.save_messages(&account.id, &messages).await?;
            Ok(())
        }
        Err(err) => {
            let message = err.to_string();
            warn!(email = %account.email, error = %message, "imap refresh failed");
            state
                .store
                .mark_account_error(&account.id, &message)
                .await?;
            Err(AppError::status(StatusCode::BAD_GATEWAY, message))
        }
    }
}

fn mail_payload(mail: StoredMail) -> MailPayload {
    MailPayload {
        id: mail.id,
        subject: mail.subject,
        text: mail.text,
        html: mail.html,
        from: mail.from_addr,
        to: mail.account_email,
        received_at: mail.received_at.to_rfc3339(),
        verification_code: mail.verification_code,
    }
}

fn import_error_dto(error: ImportError) -> ImportErrorDto {
    ImportErrorDto {
        line_number: error.line_number,
        line: error.line,
        reason: error.reason,
    }
}

fn authorize(headers: &HeaderMap, api_key: &str) -> Result<(), AppError> {
    let header = headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .and_then(|value| value.to_str().ok());
    authorize_header(header, api_key).map_err(auth_error)
}

fn authorize_x_api_key(headers: &HeaderMap, api_key: &str) -> Result<(), AppError> {
    let header = headers
        .get("x-api-key")
        .or_else(|| headers.get("authorization"))
        .and_then(|value| value.to_str().ok());
    authorize_header(header, api_key).map_err(auth_error)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(str::trim)
                .filter(|token| !token.is_empty())
        })
        .map(ToOwned::to_owned)
}

fn auth_error(error: AuthError) -> AppError {
    match error {
        AuthError::Missing => AppError::status(StatusCode::UNAUTHORIZED, "missing API key"),
        AuthError::Invalid => AppError::status(StatusCode::FORBIDDEN, "invalid API key"),
    }
}

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn status(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::status(StatusCode::INTERNAL_SERVER_ERROR, value.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Outlook Mail Pool</title>
  <style>
    :root{--bg:#f8fafc;--panel:#fff;--text:#1e293b;--muted:#64748b;--line:#e2e8f0;--primary:#2563eb;--cta:#f97316;--ok:#059669;--bad:#dc2626}
    *{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font-family:Inter,Segoe UI,Arial,sans-serif}
    main{max-width:1160px;margin:0 auto;padding:28px 20px 44px}.top{display:flex;justify-content:space-between;gap:16px;align-items:flex-start;margin-bottom:18px}
    h1{margin:0 0 6px;font-size:26px;letter-spacing:0}.sub{margin:0;color:var(--muted);font-size:14px}
    .grid{display:grid;grid-template-columns:380px minmax(0,1fr);gap:18px}.panel{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:18px;box-shadow:0 10px 28px rgba(15,23,42,.06)}
    label{display:block;margin:12px 0 6px;font-weight:650;font-size:13px}input,textarea{width:100%;border:1px solid var(--line);border-radius:6px;padding:10px 11px;font:inherit;color:var(--text);background:#fff}
    textarea{min-height:250px;resize:vertical;font-family:ui-monospace,SFMono-Regular,Consolas,monospace;font-size:13px}.actions{display:flex;gap:10px;margin-top:12px}button{border:0;border-radius:6px;padding:10px 13px;font-weight:700;cursor:pointer;background:var(--primary);color:#fff}button.secondary{background:#334155}
    .stats{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:10px;margin-bottom:14px}.stat{border:1px solid var(--line);border-radius:8px;background:#fff;padding:12px}.stat span{display:block;color:var(--muted);font-size:12px}.stat strong{font-size:22px}
    table{width:100%;border-collapse:collapse;font-size:13px}th,td{text-align:left;border-bottom:1px solid var(--line);padding:10px 8px;vertical-align:top}th{color:#475569;font-size:12px}.status{font-weight:700}.status.ready{color:var(--ok)}.status.error{color:var(--bad)}
    .msg{margin-top:10px;color:var(--muted);font-size:13px;white-space:pre-wrap}@media(max-width:860px){.grid{grid-template-columns:1fr}.stats{grid-template-columns:repeat(2,1fr)}}
  </style>
</head>
<body>
<main>
  <div class="top"><div><h1>Outlook Mail Pool</h1><p class="sub">批量导入 Outlook / Hotmail 邮箱，给 gpt-register-oss 提供验证码邮件 API。</p></div></div>
  <div class="grid">
    <section class="panel">
      <label for="key">API Key</label>
      <input id="key" type="password" autocomplete="current-password" placeholder="MAIL_POOL_API_KEY" />
      <label for="bulk">批量邮箱</label>
      <textarea id="bulk" spellcheck="false" placeholder="user@outlook.com,password&#10;user@hotmail.com:password&#10;user@outlook.com,password,outlook.office365.com,993"></textarea>
      <div class="actions"><button id="import">导入</button><button class="secondary" id="refresh">刷新</button></div>
      <div class="msg" id="message"></div>
    </section>
    <section class="panel">
      <div class="stats" id="stats"></div>
      <table><thead><tr><th>邮箱</th><th>状态</th><th>IMAP</th><th>最近检查</th><th>错误</th></tr></thead><tbody id="rows"></tbody></table>
    </section>
  </div>
</main>
<script>
const $=id=>document.getElementById(id);const msg=$("message");const key=$("key");key.value=localStorage.mailPoolKey||"";
function headers(){localStorage.mailPoolKey=key.value;return {"content-type":"application/json","authorization":"Bearer "+key.value}}
async function api(path, opts={}){const r=await fetch(path,{...opts,headers:{...headers(),...(opts.headers||{})}});const j=await r.json().catch(()=>({}));if(!r.ok)throw new Error(j.error||r.status);return j}
async function load(){try{const [s,a]=await Promise.all([api("/api/admin/stats"),api("/api/admin/accounts")]);$("stats").innerHTML=["total","idle","leased","ready","error"].map(k=>`<div class=stat><span>${k}</span><strong>${s.stats[k]??0}</strong></div>`).join("");$("rows").innerHTML=(a.accounts||[]).map(x=>`<tr><td>${x.email}</td><td class="status ${x.status}">${x.status}</td><td>${x.imap_host}:${x.imap_port}</td><td>${x.last_checked_at||"-"}</td><td>${x.last_error||""}</td></tr>`).join("");msg.textContent="已刷新"}catch(e){msg.textContent="刷新失败: "+e.message}}
$("import").onclick=async()=>{try{const r=await api("/api/admin/import",{method:"POST",body:JSON.stringify({text:$("bulk").value})});msg.textContent=`导入/更新 ${r.imported} 条，解析错误 ${r.errors.length} 条`+(r.errors.length?"\n"+r.errors.map(e=>`第${e.line_number}行: ${e.reason}`).join("\n"):"");await load()}catch(e){msg.textContent="导入失败: "+e.message}};
$("refresh").onclick=load; if(key.value) load();
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;

    use super::{AppState, router};
    use crate::{config::AppConfig, store::Store};
    use std::{net::SocketAddr, sync::Arc};
    use tokio::sync::Semaphore;

    async fn test_app() -> axum::Router {
        let store = Store::connect("sqlite::memory:").await.unwrap();
        let state = AppState {
            store,
            config: AppConfig {
                bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
                database_url: "sqlite::memory:".to_string(),
                api_key: "secret".to_string(),
                lease_ttl_seconds: 900,
                refresh_cooldown_seconds: 8,
                max_imap_workers: 2,
            },
            imap_permits: Arc::new(Semaphore::new(2)),
        };
        router(state)
    }

    #[tokio::test]
    async fn admin_import_requires_auth_and_accepts_accounts() {
        let app = test_app().await;
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/import")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"text":"a@outlook.com,p"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/import")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"text":"a@outlook.com,p"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["accepted"], 1);
    }
}
