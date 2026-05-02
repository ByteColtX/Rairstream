use std::net::UdpSocket;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tracing::{debug, trace, warn};

use super::clock::ntp_timestamp_now;
use crate::rtsp::client::map_connection_error;
use crate::session::AirPlayError;

const TIMING_POLL_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub(crate) struct TimingResponder {
    stop_requested: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TimingResponder {
    pub(crate) fn start(socket: UdpSocket) -> Result<Self, AirPlayError> {
        let local_addr = socket.local_addr().map_err(map_connection_error)?;
        socket
            .set_read_timeout(Some(TIMING_POLL_TIMEOUT))
            .map_err(map_connection_error)?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let worker = thread::Builder::new()
            .name(String::from("raop-timing-responder"))
            .spawn(move || run_timing_responder(socket, worker_stop_requested))
            .map_err(map_connection_error)?;
        debug!(local_addr = %local_addr, "Timing responder started");

        Ok(Self {
            stop_requested,
            worker: Some(worker),
        })
    }

    pub(crate) fn stop(&mut self) {
        self.stop_requested.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
            debug!("Timing responder stopped");
        }
    }
}

impl Drop for TimingResponder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_timing_responder(socket: UdpSocket, stop_requested: Arc<AtomicBool>) {
    let mut buffer = [0_u8; 1500];

    while !stop_requested.load(Ordering::Acquire) {
        match socket.recv_from(&mut buffer) {
            Ok((len, peer_addr)) => {
                trace!(peer_addr = %peer_addr, bytes = len, "received timing request");
                if let Some(reply) = build_timing_reply(&buffer[..len]) {
                    if let Err(error) = socket.send_to(&reply, peer_addr) {
                        warn!(peer_addr = %peer_addr, error = %error, "failed to send timing response");
                    } else {
                        trace!(peer_addr = %peer_addr, bytes = reply.len(), "timing response sent");
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                warn!(error = %error, "Timing responder exited due to I/O error");
                break;
            }
        }
    }

    drop(stop_requested);
    drop(socket);
}

pub(crate) fn build_timing_reply(request: &[u8]) -> Option<[u8; 32]> {
    if request.len() < 32 || request[1] & 0x7f != 82 {
        return None;
    }

    let receive_timestamp = ntp_timestamp_now();
    let transmit_timestamp = ntp_timestamp_now();
    let mut reply = [0_u8; 32];
    reply[0] = 0x80;
    reply[1] = 0x80 | 0x53;
    reply[2..4].copy_from_slice(&request[2..4]);
    reply[4..8].copy_from_slice(&request[4..8]);
    reply[8..16].copy_from_slice(&request[24..32]);
    reply[16..24].copy_from_slice(&receive_timestamp.to_be_bytes());
    reply[24..32].copy_from_slice(&transmit_timestamp.to_be_bytes());
    Some(reply)
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::time::Duration;

    use super::*;

    #[test]
    fn timing_reply_copies_sequence_and_request_timestamp() {
        let request = [
            0x80, 0xd2, 0x12, 0x34, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
        ];

        let reply = build_timing_reply(&request).unwrap();

        assert_eq!(reply[1] & 0x7f, 83);
        assert_eq!(&reply[2..4], &[0x12, 0x34]);
        assert_eq!(
            &reply[8..16],
            &[0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
        );
    }

    #[test]
    fn timing_responder_replies_to_request_packet() {
        let responder_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let responder_addr = responder_socket.local_addr().unwrap();
        let mut responder = TimingResponder::start(responder_socket).unwrap();

        let client_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        client_socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();

        let request = [
            0x80, 0xd2, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8,
        ];
        client_socket.send_to(&request, responder_addr).unwrap();

        let mut reply = [0_u8; 64];
        let (reply_len, _) = client_socket.recv_from(&mut reply).unwrap();

        responder.stop();

        assert_eq!(reply_len, 32);
        assert_eq!(reply[1] & 0x7f, 83);
        assert_eq!(&reply[2..4], &[0x00, 0x01]);
    }
}
