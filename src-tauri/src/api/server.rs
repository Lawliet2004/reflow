use std::io::Cursor;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
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

pub fn current_status(ctx: &AppContext) -> ApiStatus {
    let settings = ctx.settings_store.get();
    let running = ctx.api_runtime.read().as_ref().map(|r| r.bind.clone());
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
    let (pairing_code, expires, pair_uri_val, qr) = if let Some(offer) = offer {
        let remaining = offer
            .expires_at
            .saturating_duration_since(Instant::now())
            .as_secs();
        let uri = pair_uri(&host, settings.api_port, &offer.code);
        let svg = qr_svg(&uri);
        (Some(offer.code), Some(remaining), Some(uri), svg)
    } else {
        (None, None, None, None)
    };

    ApiStatus {
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
        warning: "LAN API is opt-in and uses HTTP on your local network. Pair only with devices you own.".into(),
    }
}

pub async fn sync_server(ctx: AppContext) -> Result<(), String> {
    let settings = ctx.settings_store.get();
    if !settings.api_enabled {
        stop_server(&ctx);
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
    stop_server(&ctx);
    start_server(ctx, bind).await
}

pub fn stop_server(ctx: &AppContext) {
    if let Some(runtime) = ctx.api_runtime.write().take() {
        let _ = runtime.shutdown.send(true);
        log::info!("Reflow LAN API stopped");
    }
}

async fn start_server(ctx: AppContext, bind: String) -> Result<(), String> {
    let listener = TcpListener::bind(&bind)
        .await
        .map_err(|e| format!("Could not bind LAN API on {bind}: {e}"))?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    *ctx.api_runtime.write() = Some(ApiRuntime {
        bind: bind.clone(),
        shutdown: shutdown_tx,
    });

    let app = router(ctx.clone());
    log::info!("Reflow LAN API listening on {bind}");
    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        });
        if let Err(err) = server.await {
            log::error!("LAN API server error: {err}");
        }
    });
    Ok(())
}

pub fn router(ctx: AppContext) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/pair", post(pair))
        .route("/v1/history", get(history).delete(clear_history))
        .route("/v1/history/search", get(search_history))
        .route("/v1/history/:id", delete(delete_history))
        .route("/v1/inject", post(inject))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/devices/:id", delete(revoke_device))
        .route("/v1/stream", get(stream_ws))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

fn bearer_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text.strip_prefix("Bearer ").or_else(|| text.strip_prefix("bearer ")) {
                return Some(token.trim().to_string());
            }
        }
    }
    query_token.map(|t| t.to_string())
}

fn require_auth(ctx: &AppContext, headers: &HeaderMap, query_token: Option<&str>) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(token) = bearer_token(headers, query_token) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"unauthorized","message":"Missing bearer token"})),
        ));
    };
    if !ctx.pairing.authorize(&token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"unauthorized","message":"Invalid token"})),
        ));
    }
    Ok(())
}

async fn health(State(ctx): State<AppContext>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: "0.1.0".into(),
        model_ready: ctx.asr_engine.read().is_model_loaded(),
        os: PlatformSys::get_system_metrics().os_name,
    })
}

async fn status(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let settings = ctx.settings_store.get();
    Ok(Json(json!({
        "state": *ctx.state_enum.read(),
        "language": settings.language,
        "model_ready": ctx.asr_engine.read().is_model_loaded(),
        "session": crate::platform::session().as_str(),
    })))
}

async fn pair(
    State(ctx): State<AppContext>,
    Json(body): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, Json<Value>)> {
    match ctx.pairing.pair(&body.code, body.device_name.as_deref().unwrap_or("Android")) {
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
    headers: HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let entries = ctx
        .history_store
        .get_entries(q.limit.unwrap_or(50), q.offset.unwrap_or(0))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!(entries)))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search_history(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let entries = ctx
        .history_store
        .search_entries(&q.q)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!(entries)))
}

async fn delete_history(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let ok = ctx
        .history_store
        .delete_entry(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!({ "deleted": ok })))
}

async fn clear_history(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let n = ctx
        .history_store
        .clear_all()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!({ "cleared": n })))
}

async fn inject(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Json(body): Json<InjectRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let settings = ctx.settings_store.get();
    // LAN API path: no captured hwnd. The Android user's desktop app is
    // expected to be foreground by the time the inject runs; skip the
    // foreground-match verification.
    let outcome = TextInjector::inject(&body.text, settings.clipboard_restore_enabled, 0)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!({
        "pasted": outcome.pasted,
        "fallback_copy": outcome.fallback_copy,
        "paste_chord": outcome.paste_chord,
    })))
}

async fn revoke_device(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
    let ok = ctx
        .pairing
        .revoke(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": e}))))?;
    Ok(Json(json!({ "revoked": ok })))
}

async fn transcribe(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, None)?;
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
    let Some(bytes) = audio else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"no_audio","message":"Missing audio file"})),
        ));
    };

    let samples = wav_or_pcm_to_f32(&bytes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"message": e})),
        )
    })?;

    session::start_external(&ctx, Some(language))
        .await
        .map_err(session_err)?;
    let _ = session::push_f32(&ctx, &samples);
    let outcome = session::stop(&ctx, inject, None).await.map_err(session_err)?;
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
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    require_auth(&ctx, &headers, q.token.as_deref())?;
    Ok(ws.on_upgrade(move |socket| handle_socket(ctx, socket)))
}

async fn handle_socket(ctx: AppContext, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut events = ctx.bus.subscribe();
    let mut sample_rate = 16000u32;
    let mut inject = ctx.settings_store.get().api_inject_default;
    let mut started = false;

    let send = |msg: ServerMsg| serde_json::to_string(&msg).unwrap_or_else(|_| "{}".into());

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMsg>(&text) {
                            Ok(ClientMsg::Start { language, sample_rate: sr, inject: inj, .. }) => {
                                if let Some(sr) = sr {
                                    sample_rate = sr;
                                }
                                if let Some(inj) = inj {
                                    inject = inj;
                                }
                                match session::start_external(&ctx, language).await {
                                    Ok(()) => {
                                        started = true;
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
                                if started {
                                    match session::stop(&ctx, inject, None).await {
                                        Ok(outcome) => {
                                            started = false;
                                            let msg = ServerMsg::Final {
                                                raw: outcome.raw,
                                                text: outcome.final_text,
                                                language: outcome.language,
                                                metrics: ctx.last_latency_metrics.read().clone(),
                                            };
                                            let _ = sink.send(Message::Text(send(msg))).await;
                                        }
                                        Err(err) => {
                                            let _ = sink.send(Message::Text(send(ServerMsg::Error {
                                                code: err.code,
                                                message: err.message,
                                            }))).await;
                                        }
                                    }
                                }
                            }
                            Ok(ClientMsg::Cancel) => {
                                let _ = session::cancel(&ctx);
                                started = false;
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
                        if started {
                            let _ = session::push_pcm_s16le(&ctx, &bin, sample_rate);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        if started {
                            let _ = session::cancel(&ctx);
                        }
                        break;
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => {
                        if started {
                            let _ = session::cancel(&ctx);
                        }
                        break;
                    }
                }
            }
            event = events.recv() => {
                match event {
                    Ok(DoryEvent::Partial(p)) if started => {
                        if sink.send(Message::Text(send(ServerMsg::from_partial(p)))).await.is_err() {
                            break;
                        }
                    }
                    Ok(DoryEvent::AutoStop) if started => {
                        // stop() is invoked by the session watch; wait for Final via REST path
                    }
                    Ok(DoryEvent::Error(message)) => {
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

fn wav_or_pcm_to_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" {
        let cursor = Cursor::new(bytes.to_vec());
        let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
        let spec = reader.spec();
        let mut samples: Vec<f32> = Vec::new();
        match spec.sample_format {
            hound::SampleFormat::Int => {
                for s in reader.samples::<i16>() {
                    samples.push(s.map_err(|e| e.to_string())? as f32 / 32768.0);
                }
            }
            hound::SampleFormat::Float => {
                for s in reader.samples::<f32>() {
                    samples.push(s.map_err(|e| e.to_string())?);
                }
            }
        }
        if spec.channels > 1 {
            let ch = spec.channels as usize;
            samples = samples
                .chunks_exact(ch)
                .map(|c| c.iter().sum::<f32>() / ch as f32)
                .collect();
        }
        if spec.sample_rate != 16000 {
            let mut resampler = crate::audio::AudioResampler::new(spec.sample_rate, 1);
            samples = resampler.resample_f32(&samples);
        }
        return Ok(samples);
    }
    Ok(crate::audio::AudioResampler::pcm16_bytes_to_f32(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatus};
    use tower::ServiceExt;

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
