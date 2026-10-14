//! Close consent requests when their owning operation is cancelled or times out.
//! Dropping a listener alone is insufficient if another task retains the bus.
use std::time::Duration;
use zbus::{Connection, Proxy};

pub(crate) struct PendingRequest {
    connection: Connection,
    path: Option<String>,
}

impl PendingRequest {
    pub(crate) fn new(connection: &Connection, path: &str) -> Self {
        Self {
            connection: connection.clone(),
            path: Some(path.into()),
        }
    }

    pub(crate) fn responded(&mut self) {
        self.path.take();
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        let connection = self.connection.clone();
        tauri::async_runtime::spawn(async move {
            let close = async {
                let proxy = Proxy::new(
                    &connection,
                    "org.freedesktop.portal.Desktop",
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                )
                .await?;
                proxy.call::<_, _, ()>("Close", &()).await
            };
            let _ = tokio::time::timeout(Duration::from_secs(1), close).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct FakeRequest(Arc<AtomicUsize>);
    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl FakeRequest {
        fn close(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[tokio::test]
    #[ignore = "run under dbus-run-session with REFLOW_PORTAL_TEST_PRIVATE_BUS=1; never use the real desktop portal"]
    async fn cancellation_closes_pending_consent_but_a_response_disarms_the_close() {
        assert_eq!(
            std::env::var("REFLOW_PORTAL_TEST_PRIVATE_BUS").as_deref(),
            Ok("1")
        );
        let closed = Arc::new(AtomicUsize::new(0));
        let path = "/org/freedesktop/portal/desktop/request/test/pending";
        let _server = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at(path, FakeRequest(Arc::clone(&closed)))
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = Connection::session().await.unwrap();
        drop(PendingRequest::new(&client, path));
        tokio::time::timeout(Duration::from_secs(2), async {
            while closed.load(Ordering::Acquire) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut answered = PendingRequest::new(&client, path);
        answered.responded();
        drop(answered);
        // Barrier: a method round trip confirms the fake bus remains usable.
        let proxy = Proxy::new(
            &client,
            "org.freedesktop.portal.Desktop",
            path,
            "org.freedesktop.portal.Request",
        )
        .await
        .unwrap();
        proxy.call::<_, _, ()>("Close", &()).await.unwrap();
        assert_eq!(closed.load(Ordering::Acquire), 2);
    }
}
