#[cfg(test)]
use std::io::Cursor;
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::net::{bind_ip, lan_ipv4_addrs, pair_uri, qr_svg};
use super::protocol::{
    ApiStatus, ClientMsg, HealthResponse, InjectRequest, PairRequest, PairResponse, ServerMsg,
};
use crate::context::{ApiRuntime, AppContext};
use crate::dory::DoryEvent;
use crate::injection::TextInjector;
use crate::platform::PlatformSys;
use crate::session::{self, SessionError};

#[cfg(test)]
#[path = "server_socket_tests.rs"]
mod socket_tests;

pub fn current_status(ctx: &AppContext) -> ApiStatus {
    let settings = ctx.settings_store.get();
    let running = ctx.api_runtime.read().as_ref().map(|r| r.bind.clone());
    let identity = if settings.api_enabled {
        super::tls::identity(ctx)
    } else {
        Err(String::new())
    };
    let fingerprint = identity
        .as_ref()
        .ok()
        .and_then(|identity| identity.fingerprint().ok());
    let offer = if settings.api_enabled {
        Some(ctx.pairing.ensure_offer())
    } else {
        ctx.pairing.current_offer()
    };
    let mut addrs = vec!["127.0.0.1".to_string()];
    if settings.api_bind != "localhost" {
        addrs.extend(lan_ipv4_addrs());
    }
    addrs.sort();
    addrs.dedup();

    let host = addrs
        .iter()
        .find(|a| *a != "127.0.0.1")
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".into());
    let (pairing_code, expires, pair_uri_val, qr) =
        if let (Some(offer), Some(fingerprint)) = (offer, fingerprint.as_deref()) {
            let remaining = offer
                .expires_at
                .saturating_duration_since(Instant::now())
                .as_secs();
            let uri = pair_uri(&host, settings.api_port, &offer.code, fingerprint);
            let svg = qr_svg(&uri);
            (Some(offer.code), Some(remaining), Some(uri), svg)
        } else {
            (None, None, None, None)
        };

    ApiStatus {
        certificate_sha256: fingerprint,
        transport: "https".into(),
        enabled: settings.api_enabled,
        running: running.is_some(),
        bind: settings.api_bind.clone(),
        port: settings.api_port,
        listen_addrs: addrs,
        pairing_code,
        pairing_expires_in_sec: expires,
        qr_svg: qr,
        pair_uri: pair_uri_val,
        devices: ctx.pairing.list_public(),
        warning: if let Err(error) = identity {
            error
        } else {
            "Phone connections use encrypted HTTPS/WSS and the desktop certificate fingerprint from this pairing link. Older companions must be updated and paired again.".into()
        },
    }
}

pub async fn sync_server(ctx: AppContext) -> Result<(), String> {
    let operation = Arc::clone(&ctx.api_operation);
    let _operation = operation.lock().await;
    let settings = ctx.settings_store.get();
    if !settings.api_enabled {
        stop_server_and_wait(&ctx).await?;
        return Ok(());
    }
    let host = bind_ip(&settings.api_bind);
    let bind = format!("{host}:{}", settings.api_port);
    if ctx
        .api_runtime
        .read()
        .as_ref()
        .map(|r| r.bind == bind)
        .unwrap_or(false)
    {
        return Ok(());
    }
    stop_server_and_wait(&ctx).await?;
    start_server(ctx, bind).await
}

pub fn stop_server(ctx: &AppContext) {
    if let Some(runtime) = ctx.api_runtime.write().take() {
        let _ = runtime.shutdown.send(true);
        log::info!("Reflow LAN API stopped");
    }
}

async fn stop_server_and_wait(ctx: &AppContext) -> Result<(), String> {
    let previous = ctx.api_runtime.write().take();
    if let Some(mut previous) = previous {
        let _ = previous.shutdown.send(true);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            previous.finished.wait_for(|done| *done),
        )
        .await
        .map_err(|_| "The previous phone listener is still closing. Retry shortly.".to_string())?
        .map_err(|_| "The previous phone listener ended unexpectedly.".to_string())?;
    }
    Ok(())
}

async fn start_server(ctx: AppContext, bind: String) -> Result<(), String> {
    let identity = super::tls::identity(&ctx)?;
    let tls_config = identity.config().await?;
    let listener = TcpListener::bind(&bind)
        .await
        .map_err(|e| format!("Could not bind LAN API on {bind}: {e}"))?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    // Keep the actual binding attached while settings and the runtime slot change.
    let localhost = bind
        .rsplit_once(':')
        .is_some_and(|(host, _)| host == "127.0.0.1" || host == "[::1]");
    let app = router_for_listener(ctx.clone(), localhost).layer(Extension(shutdown_tx.subscribe()));
    let handle = axum_server::Handle::new();
    let stop_handle = handle.clone();
    tokio::spawn(async move {
        let _ = shutdown_rx.wait_for(|stopped| *stopped).await;
        stop_handle.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
    });
    let server =
        axum_server::from_tcp_rustls(listener.into_std().map_err(|e| e.to_string())?, tls_config)
            .map_err(|e| e.to_string())?
            .handle(handle);
    static GENERATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = GENERATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (finished_tx, finished_rx) = tokio::sync::watch::channel(false);
    *ctx.api_runtime.write() = Some(ApiRuntime {
        bind: bind.clone(),
        shutdown: shutdown_tx,
        finished: finished_rx,
        generation,
    });
    log::info!("Reflow encrypted LAN API listening on {bind}");
    tokio::spawn(async move {
        if let Err(err) = server.serve(app.into_make_service()).await {
            log::error!("LAN API server error: {err}");
        }
        let _ = finished_tx.send(true);
        let mut current = ctx.api_runtime.write();
        if current
            .as_ref()
            .is_some_and(|runtime| runtime.generation == generation)
        {
            *current = None;
        }
    });
    Ok(())
}

/// At most four blocking API jobs may own worker threads. Reject overload
/// instead of queuing unbounded clipboard/database work on the async executor.
async fn blocking_job<T: Send + 'static>(
    ctx: &AppContext,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, (StatusCode, Json<Value>)> {
    let permit = Arc::clone(&ctx.api_jobs).try_acquire_owned().map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"message":"Desktop is busy. Retry shortly."})),
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        task()
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message":"Desktop operation failed."})),
        )
    })?
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message":error})),
        )
    })
}

pub fn router(ctx: AppContext) -> Router {
    let localhost = ctx.settings_store.get().api_bind == "localhost";
    router_for_listener(ctx, localhost)
}

#[derive(Clone, Copy)]
struct ListenerScope {
    localhost: bool,
}

type PhoneProcessing<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<session::StopOutcome, SessionError>> + Send + 'a>,
>;

fn router_for_listener(ctx: AppContext, localhost: bool) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/pair", post(pair))
        .route("/v1/history", get(history).delete(clear_history))
        .route("/v1/history/search", get(search_history))
        .route("/v1/history/:id", delete(delete_history))
        .route("/v1/inject", post(inject))
        .route("/v1/dictate", post(dictate))
        .route("/v1/session", post(automation_session))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/devices/:id", delete(revoke_device))
        .route("/v1/stream", get(stream_ws))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(Extension(ListenerScope { localhost }))
        .with_state(ctx)
}

fn bearer_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text
                .strip_prefix("Bearer ")
                .or_else(|| text.strip_prefix("bearer "))
            {
                return Some(token.trim().to_string());
            }
        }
    }
    query_token.map(|t| t.to_string())
}

fn require_auth(
    ctx: &AppContext,
    headers: &HeaderMap,
    query_token: Option<&str>,
    scope: ListenerScope,
) -> Result<String, (StatusCode, Json<Value>)> {
    let Some(token) = bearer_token(headers, query_token) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"unauthorized","message":"Missing bearer token"})),
        ));
    };
    if !ctx.pairing.authorize(&token)
        || (ctx.pairing.is_automation_token(&token)
            && (!scope.localhost || !automation_available(ctx)))
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"unauthorized","message":"Invalid token"})),
        ));
    }
    Ok(token)
}

fn forbidden() -> (StatusCode, Json<Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(
            json!({"code":"permission_denied","message":"Allow this permission in desktop Settings → Phone."}),
        ),
    )
}
fn require_permission(
    ctx: &AppContext,
    headers: &HeaderMap,
    query: Option<&str>,
    permission: &str,
    scope: ListenerScope,
) -> Result<String, (StatusCode, Json<Value>)> {
    let token = require_auth(ctx, headers, query, scope)?;
    let permissions = ctx.pairing.permissions(&token).ok_or_else(forbidden)?;
    let allowed = match permission {
        "stream" => permissions.stream,
        "history" => permissions.history,
        "injection" => permissions.injection,
        _ => false,
    };
    if !allowed {
        return Err(forbidden());
    }
    Ok(token)
}

async fn health(State(ctx): State<AppContext>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").into(),
        model_ready: ctx.asr_handle.is_model_loaded(),
        os: PlatformSys::get_system_metrics().os_name,
    })
}

async fn status(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None, scope)?;
    let settings = ctx.settings_store.get();
    Ok(Json(json!({
        "state": *ctx.state_enum.read(),
        "language": settings.language,
        "model_ready": ctx.asr_handle.is_model_loaded(),
        "session": crate::platform::session().as_str(),
    })))
}

async fn pair(
    State(ctx): State<AppContext>,
    Json(body): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, Json<Value>)> {
    match ctx
        .pairing
        .pair(&body.code, body.device_name.as_deref().unwrap_or("Android"))
    {
        Ok((token, _)) => {
            let settings = ctx.settings_store.get();
            Ok(Json(PairResponse {
                token,
                server_name: sysinfo::System::host_name().unwrap_or_else(|| "Reflow".into()),
                port: settings.api_port,
            }))
        }
        Err(message) => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"pair_failed","message": message})),
        )),
    }
}

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn history(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_permission(&ctx, &headers, None, "history", scope)?;
    let store = Arc::clone(&ctx.history_store);
    let entries = blocking_job(&ctx, move || {
        store.get_entries(q.limit.unwrap_or(50), q.offset.unwrap_or(0))
    })
    .await?;
    Ok(Json(json!(entries)))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search_history(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_permission(&ctx, &headers, None, "history", scope)?;
    let store = Arc::clone(&ctx.history_store);
    let entries = blocking_job(&ctx, move || store.search_entries(&q.q)).await?;
    Ok(Json(json!(entries)))
}

async fn delete_history(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None, scope)?;
    let _ = id;
    Err(forbidden())
}

async fn clear_history(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None, scope)?;
    Err(forbidden())
}

async fn inject(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Json(body): Json<InjectRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_permission(&ctx, &headers, None, "injection", scope)?;
    let settings = ctx.settings_store.get();
    // LAN API path: no captured hwnd. The Android user's desktop app is
    // expected to be foreground by the time the inject runs; skip the
    // foreground-match verification.
    if body.text.len() > 1024 * 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"message":"Text exceeds the insertion limit."})),
        ));
    }
    let outcome = blocking_job(&ctx, move || {
        TextInjector::inject(&body.text, settings.clipboard_restore_enabled, 0)
    })
    .await?;
    Ok(Json(json!({
        "pasted": outcome.pasted,
        "fallback_copy": outcome.fallback_copy,
        "paste_chord": outcome.paste_chord,
    })))
}

fn automation_available(ctx: &AppContext) -> bool {
    let settings = ctx.settings_store.get();
    settings.api_enabled
        && settings.api_bind == "localhost"
        && ctx.api_runtime.read().as_ref().is_none_or(|runtime| {
            runtime
                .bind
                .rsplit_once(':')
                .is_some_and(|(host, _)| host == "127.0.0.1" || host == "[::1]")
        })
}

fn require_automation(
    ctx: &AppContext,
    headers: &HeaderMap,
    scope: ListenerScope,
) -> Result<String, (StatusCode, Json<Value>)> {
    if !scope.localhost || !automation_available(ctx) {
        return Err(forbidden());
    }
    let token = require_auth(ctx, headers, None, scope)?;
    if !ctx.pairing.is_automation_token(&token) {
        return Err(forbidden());
    }
    Ok(token)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DictateRequest {
    text: Option<String>,
}

async fn dictate(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Json(body): Json<DictateRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_automation(&ctx, &headers, scope)?;
    let text = body.text.unwrap_or_default();
    if text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"message":"Provide nonempty text to type."})),
        ));
    }
    if text.len() > 1024 * 1024 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"message":"Text exceeds the insertion limit."})),
        ));
    }
    let restore = ctx.settings_store.get().clipboard_restore_enabled;
    let target = crate::platform::foreground_hwnd();
    let outcome = blocking_job(&ctx, move || TextInjector::inject(&text, restore, target)).await?;
    Ok(Json(
        json!({"pasted":outcome.pasted, "fallback_copy":outcome.fallback_copy, "paste_chord":outcome.paste_chord}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutomationAction {
    Start,
    Stop,
    Cancel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationSessionRequest {
    action: AutomationAction,
}

async fn automation_session(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Json(body): Json<AutomationSessionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let token = require_automation(&ctx, &headers, scope)?;
    let _permit = Arc::clone(&ctx.api_jobs).try_acquire_owned().map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"message":"Desktop is busy. Retry shortly."})),
        )
    })?;
    let hash = crate::pairing::hash_token(&token);
    // Serialize start/release requests before microphone startup finishes. The
    // token's saved session id also prevents stopping a later phone/hotkey session.
    let mut owned = ctx.pairing.automation_sessions.lock().await;
    // Recheck after waiting: desktop revocation or a bind change takes effect
    // before any queued action can touch the microphone.
    require_automation(&ctx, &headers, scope)?;
    let conflict = |message: String| (StatusCode::CONFLICT, Json(json!({"message":message})));
    match body.action {
        AutomationAction::Start => {
            if owned
                .get(&hash)
                .is_some_and(|id| Some(*id) == *ctx.current_session_id.read())
            {
                return Err(conflict("This token already owns a recording".into()));
            }
            let id = session::start_automation(&ctx)
                .await
                .map_err(|error| conflict(error.message))?;
            // Only one recording can exist; discard ended owners on each start.
            owned.clear();
            owned.insert(hash, id);
            Ok(Json(json!({"session_id": id, "state":"recording"})))
        }
        AutomationAction::Stop | AutomationAction::Cancel => {
            let id = owned
                .remove(&hash)
                .ok_or_else(|| conflict("This token has no recording".into()))?;
            if *ctx.current_session_id.read() != Some(id) {
                return Err(conflict("The owned recording has already ended".into()));
            }
            if matches!(body.action, AutomationAction::Cancel) {
                session::cancel_owned_wait(&ctx, Some(id))
                    .await
                    .map_err(|error| conflict(error.message))?;
                Ok(Json(json!({"session_id": id, "state":"cancelled"})))
            } else {
                let outcome = session::stop_owned(&ctx, true, id)
                    .await
                    .map_err(|error| conflict(error.message))?;
                Ok(Json(
                    json!({"session_id":id, "state":"ready", "text":outcome.final_text, "injected":outcome.injected}),
                ))
            }
        }
    }
}

async fn revoke_device(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None, scope)?;
    let _ = id;
    Err(forbidden())
}

async fn transcribe(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let token = require_permission(&ctx, &headers, None, "stream", scope)?;
    let mut audio: Option<Bytes> = None;
    let mut language = ctx.settings_store.get().language.clone();
    let mut inject = false;
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"message": e.to_string()})),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "language" {
            if let Ok(v) = field.text().await {
                language = v;
            }
        } else if name == "inject" {
            if let Ok(v) = field.text().await {
                inject = v == "true" || v == "1";
            }
        } else if name == "file" || name == "audio" {
            audio = field.bytes().await.ok();
        }
    }
    if inject && !ctx.pairing.permissions(&token).is_some_and(|p| p.injection) {
        return Err(forbidden());
    }
    let Some(bytes) = audio else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"no_audio","message":"Missing audio file"})),
        ));
    };

    let samples = wav_or_pcm_to_f32(&bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"message": e}))))?;

    let session_id = session::start_phone_owned(&ctx, Some(language), &token)
        .await
        .map_err(session_err)?;
    if let Err(err) = session::push_f32_owned(&ctx, session_id, &samples) {
        let _ = session::cancel_owned(&ctx, Some(session_id));
        return Err(session_err(err));
    }
    let inject = inject && ctx.pairing.permissions(&token).is_some_and(|p| p.injection);
    let outcome = session::stop_phone(&ctx, inject, session_id, &token)
        .await
        .map_err(session_err)?;
    Ok(Json(json!({
        "raw": outcome.raw,
        "text": outcome.final_text,
        "language": outcome.language,
        "injected": outcome.injected,
    })))
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn stream_ws(
    State(ctx): State<AppContext>,
    Extension(scope): Extension<ListenerScope>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    generation: Option<Extension<tokio::sync::watch::Receiver<bool>>>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let token = require_permission(&ctx, &headers, q.token.as_deref(), "stream", scope)?;
    Ok(ws
        .max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| {
            handle_socket(
                ctx,
                socket,
                token,
                scope,
                generation.map(|Extension(rx)| rx),
            )
        }))
}

fn stream_authorized(ctx: &AppContext, token: &str) -> bool {
    ctx.settings_store.get().api_enabled && ctx.pairing.permissions(token).is_some_and(|p| p.stream)
}

fn stream_authorized_for_listener(ctx: &AppContext, token: &str, scope: ListenerScope) -> bool {
    stream_authorized(ctx, token)
        && (!ctx.pairing.is_automation_token(token)
            || (scope.localhost && automation_available(ctx)))
}

fn processing_message(
    ctx: &AppContext,
    token: &str,
    scope: ListenerScope,
    outcome: Result<session::StopOutcome, SessionError>,
) -> Option<ServerMsg> {
    if !stream_authorized_for_listener(ctx, token, scope) {
        return None;
    }
    Some(match outcome {
        Ok(outcome) => ServerMsg::Final {
            raw: outcome.raw,
            text: outcome.final_text,
            language: outcome.language,
            metrics: ctx.last_latency_metrics.read().clone(),
        },
        Err(err) => ServerMsg::Error {
            code: err.code,
            message: err.message,
        },
    })
}

async fn handle_socket(
    ctx: AppContext,
    socket: WebSocket,
    token: String,
    scope: ListenerScope,
    mut generation: Option<tokio::sync::watch::Receiver<bool>>,
) {
    let (mut sink, mut stream) = socket.split();
    let mut events = ctx.bus.subscribe();
    let mut sample_rate = 16000u32;
    let mut inject = ctx.settings_store.get().api_inject_default;
    let mut started: Option<u64> = None;
    // Poll processing alongside the socket so Cancel/Close and authorization
    // changes can discard the continuation before it inserts or saves text.
    let mut processing: Option<PhoneProcessing<'_>> = None;
    let mut resampler = crate::audio::AudioResampler::new(sample_rate, 1);
    let mut auth_check = tokio::time::interval(std::time::Duration::from_secs(1));

    let send = |msg: ServerMsg| serde_json::to_string(&msg).unwrap_or_else(|_| "{}".into());

    loop {
        tokio::select! {
            biased;
            _ = async {
                if let Some(rx) = generation.as_mut() {
                    let _ = rx.wait_for(|stopped| *stopped).await;
                } else { std::future::pending::<()>().await; }
            } => break,
            _ = auth_check.tick() => {
                if !stream_authorized_for_listener(&ctx, &token, scope) {
                    break;
                }
            }
            incoming = stream.next() => {
                if !stream_authorized_for_listener(&ctx, &token, scope) {
                    break;
                }
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMsg>(&text) {
                            Ok(ClientMsg::Start { language, sample_rate: sr, inject: inj, format }) => {
                                if format.as_deref().is_some_and(|format| format != "pcm_s16le") {
                                    let _ = sink.send(Message::Text(send(ServerMsg::Error { code: "bad_format".into(), message: "Stream audio must use pcm_s16le.".into() }))).await;
                                    continue;
                                }
                                if started.is_some() {
                                    let _ = sink.send(Message::Text(send(ServerMsg::Error {
                                        code: "session_busy".into(),
                                        message: "This connection already owns a dictation session".into(),
                                    }))).await;
                                    continue;
                                }
                                let sr = sr.unwrap_or(16000);
                                if let Err(err) = session::validate_sample_rate(sr) {
                                    let _ = sink.send(Message::Text(send(ServerMsg::Error { code: "bad_sample_rate".into(), message: err.message }))).await;
                                    continue;
                                }
                                sample_rate = sr;
                                if let Some(inj) = inj {
                                    if inj && !ctx.pairing.permissions(&token).is_some_and(|p| p.injection) {
                                        let _ = sink.send(Message::Text(send(ServerMsg::Error { code: "permission_denied".into(), message: "Allow desktop insertion for this phone in desktop Settings → Phone.".into() }))).await;
                                        continue;
                                    }
                                    inject = inj;
                                }
                                match session::start_phone_owned(&ctx, language, &token).await {
                                    Ok(session_id) => {
                                        started = Some(session_id);
                                        resampler = crate::audio::AudioResampler::new(sample_rate, 1);
                                        if sink.send(Message::Text(send(ServerMsg::Ready))).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(err) => {
                                        let _ = sink.send(Message::Text(send(ServerMsg::Error {
                                            code: err.code,
                                            message: err.message,
                                        }))).await;
                                    }
                                }
                            }
                            Ok(ClientMsg::Stop) => {
                                if let Some(session_id) = started.filter(|_| processing.is_none()) {
                                    let may_inject = inject && ctx.pairing.permissions(&token).is_some_and(|p| p.injection);
                                    processing = Some(Box::pin(session::stop_phone(&ctx, may_inject, session_id, &token)));
                                }
                            }
                            Ok(ClientMsg::Cancel) => {
                                // Drop stop_phone first to release session_operation.
                                processing = None;
                                if let Some(session_id) = started.take() {
                                    let _ = session::cancel_owned_wait(&ctx, Some(session_id)).await;
                                }
                            }
                            Err(err) => {
                                let _ = sink.send(Message::Text(send(ServerMsg::Error {
                                    code: "bad_message".into(),
                                    message: err.to_string(),
                                }))).await;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(bin))) => {
                        if processing.is_some() { continue; }
                        if let Some(session_id) = started {
                            if *ctx.current_session_id.read() != Some(session_id) {
                                // Keep the owner until its queued completion is
                                // delivered; late audio must not swallow Final.
                                continue;
                            }
                            if bin.len() % 2 != 0 {
                                let _ = sink.send(Message::Text(send(ServerMsg::Error { code: "bad_audio".into(), message: "PCM audio must contain complete 16-bit samples".into() }))).await;
                                continue;
                            }
                            let samples = crate::audio::AudioResampler::pcm16_bytes_to_f32(&bin);
                            let samples = resampler.resample_f32(&samples);
                            if let Err(err) = session::push_f32_owned(&ctx, session_id, &samples) {
                                let _ = sink.send(Message::Text(send(ServerMsg::Error { code: err.code, message: err.message }))).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => {
                        break;
                    }
                }
            }
            outcome = async {
                match processing.as_mut() {
                    Some(stop) => stop.await,
                    None => std::future::pending().await,
                }
            } => {
                processing = None;
                let Some(message) = processing_message(&ctx, &token, scope, outcome) else { break; };
                started = None;
                if sink.send(Message::Text(send(message))).await.is_err() { break; }
            }
            event = events.recv() => {
                if !stream_authorized_for_listener(&ctx, &token, scope) {
                    break;
                }
                match event {
                    Ok(DoryEvent::Partial(p)) if started.is_some() && *ctx.current_session_id.read() == started => {
                        if sink.send(Message::Text(send(ServerMsg::from_partial(p)))).await.is_err() {
                            break;
                        }
                    }
                    Ok(DoryEvent::SessionFinished { session_id, raw, text, language, metrics }) if started == Some(session_id) && processing.is_none() => {
                        started = None;
                        let _ = sink.send(Message::Text(send(ServerMsg::Final { raw, text, language, metrics }))).await;
                    }
                    Ok(DoryEvent::Error(message)) if started.is_some() => {
                        let _ = sink.send(Message::Text(send(ServerMsg::Error {
                            code: "error".into(),
                            message,
                        }))).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        }
    }
    // Inference cancellation must run after the processing future releases its lock.
    drop(processing);
    if let Some(session_id) = started {
        let _ = session::cancel_owned_wait(&ctx, Some(session_id)).await;
    }
}

fn session_err(err: SessionError) -> (StatusCode, Json<Value>) {
    let status = if err.code == "session_busy" {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(json!({"code": err.code, "message": err.message})),
    )
}

use crate::audio::file_decode::wav_or_pcm_to_f32;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatus};
    use tower::ServiceExt;

    #[tokio::test]
    async fn retired_lan_listener_never_accepts_local_automation_after_settings_change() {
        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_retired_lan_{}", uuid::Uuid::new_v4())),
        );
        ctx.settings_store
            .merge_update(json!({"api_enabled":true,"api_bind":"lan"}))
            .unwrap();
        let (token, _) = ctx.pairing.create_automation_token().unwrap();
        let old_listener = router(ctx.clone());
        // stop_server_and_wait takes the runtime before the old listener closes.
        ctx.settings_store
            .merge_update(json!({"api_bind":"localhost"}))
            .unwrap();
        assert!(ctx.api_runtime.read().is_none());
        let _all = Arc::clone(&ctx.api_jobs)
            .acquire_many_owned(4)
            .await
            .unwrap();
        for (route, body) in [
            ("/v1/dictate", r#"{"text":"must not paste"}"#),
            ("/v1/session", r#"{"action":"cancel"}"#),
        ] {
            let response = old_listener
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(route)
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                HttpStatus::FORBIDDEN,
                "{route} must retain its listener's LAN restriction"
            );
        }
        let response = old_listener
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn automation_stale_owner_cannot_cancel_later_phone_session() {
        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_automation_stale_{}", uuid::Uuid::new_v4())),
        );
        ctx.settings_store
            .merge_update(json!({"api_enabled":true,"api_bind":"localhost"}))
            .unwrap();
        let (token, _) = ctx.pairing.create_automation_token().unwrap();
        ctx.pairing
            .automation_sessions
            .lock()
            .await
            .insert(crate::pairing::hash_token(&token), 70);
        *ctx.current_session_id.write() = Some(71);
        *ctx.capture_kind.write() = crate::dory::CaptureKind::External;
        *ctx.state_enum.write() = crate::state::AppStateEnum::Recording;
        let response = router(ctx.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/session")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"cancel"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::CONFLICT);
        assert_eq!(*ctx.current_session_id.read(), Some(71));
        assert_eq!(*ctx.capture_kind.read(), crate::dory::CaptureKind::External);
        assert_eq!(
            *ctx.state_enum.read(),
            crate::state::AppStateEnum::Recording
        );
    }

    #[tokio::test]
    async fn automation_rejects_lan_listener_during_localhost_settings_transition() {
        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_automation_bind_{}", uuid::Uuid::new_v4())),
        );
        ctx.settings_store
            .merge_update(json!({"api_enabled":true,"api_bind":"localhost"}))
            .unwrap();
        let (token, _) = ctx.pairing.create_automation_token().unwrap();
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let (_, finished) = tokio::sync::watch::channel(false);
        *ctx.api_runtime.write() = Some(ApiRuntime {
            bind: "0.0.0.0:7840".into(),
            shutdown,
            finished,
            generation: 1,
        });
        let response = router(ctx.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/dictate")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"must not paste"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::FORBIDDEN);
    }

    #[tokio::test]
    async fn paired_stream_clients_cannot_read_history_or_manage_desktop_data() {
        let dir =
            std::env::temp_dir().join(format!("reflow_api_permissions_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir);
        let offer = ctx.pairing.rotate_code();
        let (token, _) = ctx.pairing.pair(&offer.code, "stream phone").unwrap();
        let offer = ctx.pairing.rotate_code();
        let (other_token, other) = ctx.pairing.pair(&offer.code, "other phone").unwrap();
        for (method, uri) in [
            ("GET", "/v1/history".to_string()),
            ("GET", "/v1/history/search?q=private".to_string()),
            ("DELETE", "/v1/history".to_string()),
            ("DELETE", "/v1/history/unknown".to_string()),
            ("DELETE", format!("/v1/devices/{}", other.id)),
        ] {
            let response = router(ctx.clone())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                HttpStatus::FORBIDDEN,
                "{method} must require explicit desktop permission"
            );
        }
        assert!(ctx.pairing.authorize(&other_token));
    }

    #[tokio::test]
    async fn paired_stream_clients_cannot_enter_clipboard_work() {
        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_api_inject_perm_{}", uuid::Uuid::new_v4())),
        );
        let offer = ctx.pairing.rotate_code();
        let (token, _) = ctx.pairing.pair(&offer.code, "stream phone").unwrap();
        // Saturate jobs so a broken permission gate cannot touch the real clipboard.
        let _all = Arc::clone(&ctx.api_jobs)
            .acquire_many_owned(4)
            .await
            .unwrap();
        let response = router(ctx)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/inject")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"private text"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::FORBIDDEN);
    }

    #[tokio::test]
    async fn paired_stream_clients_cannot_request_injection_via_uploaded_audio() {
        let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
            "reflow_api_uploaded_inject_perm_{}",
            uuid::Uuid::new_v4()
        )));
        let offer = ctx.pairing.rotate_code();
        let (token, _) = ctx.pairing.pair(&offer.code, "stream phone").unwrap();
        // The invalid WAV proves authorization happens before decoding or
        // starting an ASR session, without ever accessing a model or clipboard.
        let body = concat!(
            "--permission-boundary\r\n",
            "Content-Disposition: form-data; name=\"inject\"\r\n\r\n",
            "true\r\n",
            "--permission-boundary\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"test.wav\"\r\n",
            "Content-Type: audio/wav\r\n\r\n",
            "RIFFinvalid-WAV\r\n",
            "--permission-boundary--\r\n"
        );
        let response = router(ctx.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/transcribe")
                    .header("authorization", format!("Bearer {token}"))
                    .header(
                        "content-type",
                        "multipart/form-data; boundary=permission-boundary",
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::FORBIDDEN);
        assert!(ctx.current_session_id.read().is_none());
    }

    #[tokio::test]
    async fn history_opt_in_allows_reads_without_remote_data_management() {
        let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
            "reflow_api_history_read_only_{}",
            uuid::Uuid::new_v4()
        )));
        let offer = ctx.pairing.rotate_code();
        let (token, device) = ctx.pairing.pair(&offer.code, "history phone").unwrap();
        ctx.pairing
            .set_permissions(
                &device.id,
                crate::pairing::DevicePermissions {
                    stream: true,
                    history: true,
                    injection: false,
                },
            )
            .unwrap();
        for (method, uri, expected) in [
            ("GET", "/v1/history".to_string(), HttpStatus::OK),
            (
                "GET",
                "/v1/history/search?q=private".to_string(),
                HttpStatus::OK,
            ),
            ("DELETE", "/v1/history".to_string(), HttpStatus::FORBIDDEN),
            (
                "DELETE",
                "/v1/history/unknown".to_string(),
                HttpStatus::FORBIDDEN,
            ),
            (
                "DELETE",
                format!("/v1/devices/{}", device.id),
                HttpStatus::FORBIDDEN,
            ),
        ] {
            let response = router(ctx.clone())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{method} read-only permission");
        }
        assert!(ctx.pairing.authorize(&token));
    }

    #[tokio::test]
    async fn real_listener_requires_tls_and_the_trusted_desktop_certificate() {
        let dir =
            std::env::temp_dir().join(format!("reflow_tls_listener_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir);
        ctx.settings_store
            .merge_update(json!({"api_enabled": true}))
            .unwrap();
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reserved.local_addr().unwrap();
        drop(reserved);
        start_server(ctx.clone(), address.to_string())
            .await
            .unwrap();
        let identity = super::super::tls::identity(&ctx).unwrap();
        let trusted = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(identity.certificate_pem.as_bytes()).unwrap(),
            )
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        assert!(trusted
            .get(format!("https://{address}/v1/health"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        assert!(reqwest::Client::new()
            .get(format!("https://{address}/v1/health"))
            .send()
            .await
            .is_err());
        let plaintext = trusted
            .get(format!("http://{address}/v1/health"))
            .send()
            .await;
        assert!(plaintext.is_err() || !plaintext.unwrap().status().is_success());
        stop_server_and_wait(&ctx).await.unwrap();
        assert!(ctx.api_runtime.read().is_none());
    }

    #[tokio::test]
    async fn api_blocking_work_rejects_overload_without_parking_async_workers() {
        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_api_jobs_{}", uuid::Uuid::new_v4())),
        );
        let _all = Arc::clone(&ctx.api_jobs)
            .acquire_many_owned(4)
            .await
            .unwrap();
        let failure = blocking_job(&ctx, || Ok(1)).await.unwrap_err();
        assert_eq!(failure.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn stream_authorization_rechecks_revocation_and_api_disable() {
        let dir =
            std::env::temp_dir().join(format!("reflow_api_stream_auth_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        ctx.settings_store
            .merge_update(json!({"api_enabled": true}))
            .unwrap();
        let offer = ctx.pairing.rotate_code();
        let (token, device) = ctx.pairing.pair(&offer.code, "phone").unwrap();
        assert!(stream_authorized(&ctx, &token));
        ctx.settings_store
            .merge_update(json!({"api_enabled": false}))
            .unwrap();
        assert!(!stream_authorized(&ctx, &token));
        ctx.settings_store
            .merge_update(json!({"api_enabled": true}))
            .unwrap();
        ctx.pairing.revoke(&device.id).unwrap();
        assert!(!stream_authorized(&ctx, &token));
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn established_idle_websocket_closes_after_revocation_or_disable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for action in [0, 1, 2, 3] {
            let dir =
                std::env::temp_dir().join(format!("reflow_api_ws_close_{}", uuid::Uuid::new_v4()));
            let ctx = AppContext::bootstrap_test(dir.clone());
            ctx.settings_store
                .merge_update(json!({"api_enabled": true}))
                .unwrap();
            let offer = ctx.pairing.rotate_code();
            let (token, device) = ctx.pairing.pair(&offer.code, "phone").unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (generation_tx, generation_rx) = tokio::sync::watch::channel(false);
            let app = router(ctx.clone()).layer(Extension(generation_rx));
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut connection = tokio::net::TcpStream::connect(addr).await.unwrap();
            let handshake = format!(
                "GET /v1/stream HTTP/1.1\r\nHost: {addr}\r\n\
                 Connection: Upgrade\r\nUpgrade: websocket\r\n\
                 Sec-WebSocket-Version: 13\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                 Authorization: Bearer {token}\r\n\r\n"
            );
            connection.write_all(handshake.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !response.ends_with(b"\r\n\r\n") {
                    response.push(connection.read_u8().await.unwrap());
                }
            })
            .await
            .expect("WebSocket handshake should finish");
            assert!(response.starts_with(b"HTTP/1.1 101"));
            if action == 0 {
                ctx.pairing.revoke(&device.id).unwrap();
            } else if action == 1 {
                ctx.settings_store
                    .merge_update(json!({"api_enabled": false}))
                    .unwrap();
            } else if action == 2 {
                assert!(stream_authorized(&ctx, &token));
                generation_tx.send(true).unwrap();
            } else {
                ctx.pairing
                    .set_permissions(
                        &device.id,
                        crate::pairing::DevicePermissions {
                            stream: false,
                            history: false,
                            injection: false,
                        },
                    )
                    .unwrap();
                assert!(ctx.pairing.authorize(&token));
            }
            let mut byte = [0u8; 1];
            let read = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                connection.read(&mut byte),
            )
            .await
            .expect("Revoked or disabled idle connection must close")
            .unwrap();
            assert_eq!(read, 0);
            server.abort();
            let _ = server.await;
            drop(connection);
            drop(ctx);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn wav_rejects_unreasonable_sample_rate_before_resampling() {
        let mut bytes = Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 1,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
            writer.write_sample(100i16).unwrap();
            writer.write_sample(100i16).unwrap();
            writer.finalize().unwrap();
        }
        assert!(wav_or_pcm_to_f32(bytes.get_ref()).is_err());
    }

    #[tokio::test]
    async fn websocket_rejects_unsupported_format_before_starting_asr() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let ctx = AppContext::bootstrap_test(
            std::env::temp_dir().join(format!("reflow_api_ws_format_{}", uuid::Uuid::new_v4())),
        );
        ctx.settings_store
            .merge_update(json!({"api_enabled": true}))
            .unwrap();
        let offer = ctx.pairing.rotate_code();
        let (token, _) = ctx.pairing.pair(&offer.code, "format phone").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(ctx.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut connection = tokio::net::TcpStream::connect(addr).await.unwrap();
        let handshake = format!(
            "GET /v1/stream HTTP/1.1\r\nHost: {addr}\r\n\
             Connection: Upgrade\r\nUpgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Authorization: Bearer {token}\r\n\r\n"
        );
        connection.write_all(handshake.as_bytes()).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(3), async {
            let mut response = Vec::new();
            while !response.ends_with(b"\r\n\r\n") {
                response.push(connection.read_u8().await.unwrap());
            }
            response
        })
        .await
        .expect("WebSocket handshake must finish");
        assert!(response.starts_with(b"HTTP/1.1 101"));

        let payload = br#"{"type":"start","format":"mp3","sample_rate":16000}"#;
        let mask = [1, 2, 3, 4];
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ mask[i % 4]),
        );
        connection.write_all(&frame).await.unwrap();
        let message = tokio::time::timeout(Duration::from_secs(3), async {
            let opcode = connection.read_u8().await.unwrap();
            assert_eq!(opcode & 0x0f, 1, "Expected text error frame");
            let length = connection.read_u8().await.unwrap();
            assert_eq!(length & 0x80, 0, "Server frames must be unmasked");
            let length = match length & 0x7f {
                126 => connection.read_u16().await.unwrap() as usize,
                127 => connection.read_u64().await.unwrap() as usize,
                length => length as usize,
            };
            assert!(length <= 4096, "Error frame must remain bounded");
            let mut data = vec![0; length];
            connection.read_exact(&mut data).await.unwrap();
            serde_json::from_slice::<Value>(&data).unwrap()
        })
        .await
        .expect("Unsupported format must fail promptly");
        assert_eq!(message["type"], "error");
        assert_eq!(message["code"], "bad_format");
        assert!(ctx.current_session_id.read().is_none());
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn health_is_public() {
        let dir = std::env::temp_dir().join(format!("reflow_api_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let app = router(ctx);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::OK);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn status_requires_auth() {
        let dir = std::env::temp_dir().join(format!("reflow_api2_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let app = router(ctx);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn pair_then_status() {
        let dir = std::env::temp_dir().join(format!("reflow_api3_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let offer = ctx.pairing.rotate_code();
        let app = router(ctx.clone());
        let body = serde_json::json!({"code": offer.code, "device_name": "Pixel"});
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: PairResponse = serde_json::from_slice(&bytes).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header("authorization", format!("Bearer {}", parsed.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), HttpStatus::OK);
        let _ = std::fs::remove_dir_all(dir);
    }
}
