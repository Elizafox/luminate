// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::{
    Address, Connection, EVENT_PROTOCOL_VERSION, Error, Event, EventCompatibility, IO_TIMEOUT,
    Path, ProtocolEvent, Result, SubscribeAck, SubscribeHello, connect, fmt, receive, send,
    timeout,
};
use luminate_protocol::EventTicket;

/// A registered read-only event stream from the daemon.
pub struct EventSubscription {
    stream: Connection,
    /// Set before reading a frame and cleared only after the complete frame is
    /// decoded. Cancellation or an error therefore leaves the stream poisoned.
    poisoned: bool,
}

impl fmt::Debug for EventSubscription {
    // The type-erased connection does not implement Debug.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventSubscription")
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl EventSubscription {
    /// Connects directly to an event socket and completes its subscribe
    /// handshake.
    ///
    /// # Errors
    ///
    /// Returns a connection, framing, timeout, or compatibility error if the
    /// event handshake cannot be completed safely.
    pub(crate) async fn connect_path_with_ticket(
        path: impl AsRef<Path>,
        ticket: EventTicket,
    ) -> Result<Self> {
        let mut stream = connect(&Address::from_configured_path(path.as_ref())).await?;
        timeout(
            IO_TIMEOUT,
            send(
                &mut stream,
                &SubscribeHello::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), ticket),
            ),
        )
        .await??;
        let ack: SubscribeAck = timeout(IO_TIMEOUT, receive(&mut stream)).await??;
        let daemon_version = ack.daemon_version.clone();
        match ack.compatibility {
            EventCompatibility::Compatible => {
                if ack.event_protocol_version != EVENT_PROTOCOL_VERSION {
                    return Err(Error::IncompatibleEventSocket {
                        daemon_version,
                        supported_event_protocol_version: ack.event_protocol_version,
                        reason: Some(format!(
                            "daemon reported compatibility but advertised event protocol {}, \
                             which does not match this client's {EVENT_PROTOCOL_VERSION}",
                            ack.event_protocol_version
                        )),
                    });
                }
                Ok(Self {
                    stream,
                    poisoned: false,
                })
            }
            EventCompatibility::Incompatible {
                supported_event_protocol_version,
                reason,
            } => Err(Error::IncompatibleEventSocket {
                daemon_version,
                supported_event_protocol_version,
                reason,
            }),
        }
    }

    /// Waits until the daemon reports an event.
    ///
    /// This intentionally has no idle timeout: a healthy topology may remain
    /// unchanged indefinitely. If this future is cancelled or frame I/O fails,
    /// the subscription is poisoned and must be replaced because the stream may
    /// be positioned partway through a frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConnectionPoisoned`] after a prior incomplete read, or
    /// an I/O/protocol error if this frame cannot be received completely.
    pub async fn next_event(&mut self) -> Result<Event> {
        if self.poisoned {
            return Err(Error::ConnectionPoisoned);
        }
        self.poisoned = true;
        let event: ProtocolEvent = receive(&mut self.stream).await?;
        self.poisoned = false;
        Ok(event.into())
    }
}
