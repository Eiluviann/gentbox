//! Gentbox phone relay: the [`Relay`] trait the hub dispatches phone
//! deliveries to, and [`NoopRelay`] for tests.
//!
//! v1 is desktop-only (architecture §0): no adapter is registered and the hub
//! holds no relay, so nothing here runs. The `apns` and `ntfy` adapters, the
//! answer stream and the payload modes arrive with the mobile release (M1),
//! which may widen this trait. Knows nothing of the hub.

use std::future::Future;
use std::pin::Pin;

use gentbox_core::ids::{MessageId, ThreadId};
use thiserror::Error;

/// A future a relay returns; boxed so the trait stays object-safe.
pub type RelayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RelayError>> + Send + 'a>>;

/// What the phone is sent for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneNotification {
    /// The hub-assigned message id.
    pub message_id: MessageId,
    /// The thread the notification groups under.
    pub thread_id: ThreadId,
    /// The type's title phrase ("Your input is needed").
    pub title: String,
    /// "Urgent · Waiting on you · topic · thread", when there is one.
    pub subtitle: Option<String>,
    /// The message summary, else its title.
    pub body: String,
}

/// What a relay hands back for a delivered notification, to update or clear
/// it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayReceipt {
    /// The message it was delivered for.
    pub message_id: MessageId,
    /// The adapter's own handle (APNs collapse id, ntfy sequence id).
    pub handle: String,
}

/// A failed relay call.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelayError {
    /// The service could not be reached or refused the request.
    #[error("relay delivery failed: {0}")]
    Delivery(String),
}

/// A phone channel: delivers, updates and clears notifications.
pub trait Relay: Send + Sync {
    /// The adapter's name, for logs (`apns`, `ntfy`, `noop`).
    fn name(&self) -> &str;

    /// Delivers a notification.
    fn publish(&self, notification: PhoneNotification) -> RelayFuture<'_, RelayReceipt>;

    /// Replaces a delivered notification in place.
    fn update(
        &self,
        receipt: &RelayReceipt,
        notification: PhoneNotification,
    ) -> RelayFuture<'_, RelayReceipt>;

    /// Removes a delivered notification.
    fn clear(&self, receipt: &RelayReceipt) -> RelayFuture<'_, ()>;
}

/// A relay that accepts everything and delivers nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopRelay;

impl Relay for NoopRelay {
    fn name(&self) -> &str {
        "noop"
    }

    fn publish(&self, notification: PhoneNotification) -> RelayFuture<'_, RelayReceipt> {
        Box::pin(async move {
            Ok(RelayReceipt {
                handle: notification.message_id.to_string(),
                message_id: notification.message_id,
            })
        })
    }

    fn update(
        &self,
        receipt: &RelayReceipt,
        _notification: PhoneNotification,
    ) -> RelayFuture<'_, RelayReceipt> {
        let receipt = receipt.clone();
        Box::pin(async move { Ok(receipt) })
    }

    fn clear(&self, _receipt: &RelayReceipt) -> RelayFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
