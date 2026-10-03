use axum::body::Body;
use axum::http::{Request, StatusCode};
use reflow_lib::{api, context::AppContext, pairing::PairingState, platform::PlatformSys};
use serde_json::json;
use tower::ServiceExt;

fn context() -> AppContext {
    AppContext::bootstrap_test(
        std::env::temp_dir().join(format!("reflow_automation_{}", uuid::Uuid::new_v4())),
    )
}

async fn post(ctx: AppContext, route: &str, token: &str, body: serde_json::Value) -> StatusCode {
    api::server::router(ctx)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[test]
fn automation_tokens_are_hashed_rotatable_and_revocable() {
    let directory = std::env::temp_dir().join(format!("reflow_token_{}", uuid::Uuid::new_v4()));
    let path = directory.join("devices.json");
    let store = PairingState::new(path.clone());
    let (first, device) = store.create_automation_token().unwrap();
    assert!(
        device.permissions.stream && device.permissions.history && device.permissions.injection
    );
    assert!(!std::fs::read_to_string(&path).unwrap().contains(&first));
    assert!(
        store.list_public().is_empty(),
        "automation is managed separately from phones"
    );
    assert!(PairingState::new(path.clone()).is_automation_token(&first));
    let (second, device) = store.create_automation_token().unwrap();
    assert!(!store.authorize(&first));
    assert!(store.is_automation_token(&second));
    store.revoke(&device.id).unwrap();
    assert!(!store.authorize(&second));
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn automation_routes_reject_phone_tokens_before_side_effects() {
    let ctx = context();
    ctx.settings_store
        .merge_update(json!({"api_enabled":true,"api_bind":"localhost"}))
        .unwrap();
    let offer = ctx.pairing.rotate_code();
    let (token, device) = ctx.pairing.pair(&offer.code, "phone").unwrap();
    ctx.pairing
        .set_permissions(
            &device.id,
            reflow_lib::pairing::DevicePermissions {
                stream: true,
                history: true,
                injection: true,
            },
        )
        .unwrap();
    for (route, body) in [
        ("/v1/dictate", json!({"text":"do not paste"})),
        ("/v1/session", json!({"action":"start"})),
    ] {
        assert_eq!(
            post(ctx.clone(), route, &token, body).await,
            StatusCode::FORBIDDEN
        );
    }
    assert!(ctx.current_session_id.read().is_none());
}

#[tokio::test]
async fn automation_credentials_stop_working_on_lan_and_when_disabled() {
    let ctx = context();
    let (token, _) = ctx.pairing.create_automation_token().unwrap();
    for settings in [
        json!({"api_enabled":false,"api_bind":"localhost"}),
        json!({"api_enabled":true,"api_bind":"lan"}),
    ] {
        ctx.settings_store.merge_update(settings).unwrap();
        assert_eq!(
            post(
                ctx.clone(),
                "/v1/dictate",
                &token,
                json!({"text":"do not paste"})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        let response = api::server::router(ctx.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/history")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn automation_validates_text_and_cannot_stop_unowned_session() {
    let ctx = context();
    ctx.settings_store
        .merge_update(json!({"api_enabled":true,"api_bind":"localhost"}))
        .unwrap();
    let (token, _) = ctx.pairing.create_automation_token().unwrap();
    let _all = ctx.api_jobs.clone().acquire_many_owned(4).await.unwrap();
    assert_eq!(
        post(
            ctx.clone(),
            "/v1/session",
            "invalid-token",
            json!({"action":"start"})
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            ctx.clone(),
            "/v1/dictate",
            "invalid-token",
            json!({"text":"do not paste"})
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(ctx.clone(), "/v1/dictate", &token, json!({})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            ctx.clone(),
            "/v1/dictate",
            &token,
            json!({"text":"x".repeat(1024*1024+1)})
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        post(
            ctx.clone(),
            "/v1/dictate",
            &token,
            json!({"text":"valid text"})
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(_all);
    *ctx.current_session_id.write() = Some(71);
    for action in ["stop", "cancel"] {
        assert_eq!(
            post(ctx.clone(), "/v1/session", &token, json!({"action":action})).await,
            StatusCode::CONFLICT
        );
        assert_eq!(*ctx.current_session_id.read(), Some(71));
    }
}

#[test]
fn portable_directory_is_beside_executable() {
    let directory = std::env::temp_dir().join("portable application");
    assert_eq!(
        PlatformSys::portable_dir_for_executable(&directory.join("reflow.exe")).unwrap(),
        directory.join("reflow-data")
    );
    assert!(PlatformSys::portable_dir_for_executable(std::path::Path::new("reflow.exe")).is_err());
}
