// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use crate::types::BadPeer;
use sc_network::ReputationChange as Rep;
use sc_network_types::PeerId;
use schnellru::{ByLength, LruMap};

const LOG_TARGET: &str = "sync::disconnected_peers";

/// The maximum number of disconnected peers to keep track of.
///
/// When a peer disconnects, we must keep track if it was in the middle of a request.
/// The peer may disconnect because it cannot keep up with the number of requests
/// (ie not having enough resources available to handle the requests); or because it is malicious.
const MAX_DISCONNECTED_PEERS_STATE: u32 = 512;

/// The time we are going to backoff a peer that has disconnected with an inflight request.
///
/// The backoff time is calculated as `num_disconnects * DISCONNECTED_PEER_BACKOFF_SECONDS`.
/// This is to prevent submitting a request to a peer that has disconnected because it could not
/// keep up with the number of requests.
///
/// The peer may disconnect due to the keep-alive timeout, however disconnections without
/// an inflight request are not tracked.
const DISCONNECTED_PEER_BACKOFF_SECONDS: u64 = 60;

/// Maximum number of disconnects with a request in flight before a peer is banned.
///
/// HARLEQUIN PATCH (, ticket 3154fe0d): unused for banning — see
/// `on_disconnect_during_request`. Kept for the state-reset threshold below.
const MAX_NUM_DISCONNECTS: u64 = 3;

/// HARLEQUIN PATCH: ceiling for the escalating backoff multiplier, so a long-flapping peer is
/// retried at most every `10 * DISCONNECTED_PEER_BACKOFF_SECONDS` (10 min) instead of unboundedly.
const MAX_BACKOFF_MULTIPLIER: u64 = 10;

/// Peer disconnected with a request in flight after backoffs.
///
/// The peer may be slow to respond to the request after backoffs, or it refuses to respond.
/// Report the peer and let the reputation system handle disconnecting the peer.
pub const REPUTATION_REPORT: Rep = Rep::new_fatal("Peer disconnected with inflight after backoffs");

/// The state of a disconnected peer with a request in flight.
#[derive(Debug)]
struct DisconnectedState {
	/// The total number of disconnects.
	num_disconnects: u64,
	/// The time at the last disconnect.
	last_disconnect: std::time::Instant,
}

impl DisconnectedState {
	/// Create a new `DisconnectedState`.
	pub fn new() -> Self {
		Self { num_disconnects: 1, last_disconnect: std::time::Instant::now() }
	}

	/// Increment the number of disconnects.
	pub fn increment(&mut self) {
		self.num_disconnects = self.num_disconnects.saturating_add(1);
		self.last_disconnect = std::time::Instant::now();
	}

	/// Get the number of disconnects.
	pub fn num_disconnects(&self) -> u64 {
		self.num_disconnects
	}

	/// Get the time of the last disconnect.
	pub fn last_disconnect(&self) -> std::time::Instant {
		self.last_disconnect
	}
}

/// Tracks the state of disconnected peers with a request in flight.
///
/// This helps to prevent submitting requests to peers that have disconnected
/// before responding to the request to offload the peer.
pub struct DisconnectedPeers {
	/// The state of disconnected peers.
	disconnected_peers: LruMap<PeerId, DisconnectedState>,
	/// Backoff duration in seconds.
	backoff_seconds: u64,
}

impl DisconnectedPeers {
	/// Create a new `DisconnectedPeers`.
	pub fn new() -> Self {
		Self {
			disconnected_peers: LruMap::new(ByLength::new(MAX_DISCONNECTED_PEERS_STATE)),
			backoff_seconds: DISCONNECTED_PEER_BACKOFF_SECONDS,
		}
	}

	/// Insert a new peer to the persistent state if not seen before, or update the state if seen.
	///
	/// Returns true if the peer should be disconnected.
	pub fn on_disconnect_during_request(&mut self, peer: PeerId) -> Option<BadPeer> {
		if let Some(state) = self.disconnected_peers.get(&peer) {
			state.increment();

			let over_threshold = state.num_disconnects() >= MAX_NUM_DISCONNECTS;
			log::debug!(
				target: LOG_TARGET,
				"Disconnected known peer {peer} state: {state:?}, over threshold: {over_threshold}",
			);

			// HARLEQUIN PATCH (, ticket 3154fe0d): NEVER escalate to the fatal-reputation
			// BAN. Upstream reports `REPUTATION_REPORT` (fatal) after MAX_NUM_DISCONNECTS, which
			// disconnects and bans the peer — including RESERVED peers (the reserved set only
			// guarantees redial after expiry, not scoring immunity). On our small mesh this turned
			// every flapping home link into a ban loop in BOTH directions (bootnode↔ct103, tablet).
			// The sync-availability backoff in `is_peer_available` (escalating, capped) already
			// protects this node from wasting requests on a flapping peer; the CONNECTION should
			// survive. Strangers remain covered by every other scoring path (keep-alive, bad
			// blocks, protocol violations) — this only removes the flap-equals-malice equation.
			None::<BadPeer>
		} else {
			log::debug!(
				target: LOG_TARGET,
				"Added peer {peer} for the first time"
			);
			// First time we see this peer.
			self.disconnected_peers.insert(peer, DisconnectedState::new());
			None
		}
	}

	/// Check if a peer is available for queries.
	pub fn is_peer_available(&mut self, peer_id: &PeerId) -> bool {
		let Some(state) = self.disconnected_peers.get(peer_id) else {
			return true;
		};

		let elapsed = state.last_disconnect().elapsed();
		let multiplier = state.num_disconnects.min(MAX_BACKOFF_MULTIPLIER); // HARLEQUIN PATCH: cap
		if elapsed.as_secs() >= self.backoff_seconds * multiplier {
			log::debug!(target: LOG_TARGET, "Peer {peer_id} is available for queries");
			self.disconnected_peers.remove(peer_id);
			true
		} else {
			log::debug!(target: LOG_TARGET,"Peer {peer_id} is backedoff");
			false
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	#[test]
	fn test_disconnected_peer_state() {
		let mut state = DisconnectedPeers::new();
		let peer = PeerId::random();

		// Is not part of the disconnected peers yet.
		assert_eq!(state.is_peer_available(&peer), true);

		for _ in 0..MAX_NUM_DISCONNECTS - 1 {
			assert!(state.on_disconnect_during_request(peer).is_none());
			assert_eq!(state.is_peer_available(&peer), false);
		}

		// HARLEQUIN PATCH: crossing the threshold must NOT ban — the peer stays tracked and
		// backed off (flap is not malice on a reserved-mesh network).
		assert!(state.on_disconnect_during_request(peer).is_none());
		assert!(state.disconnected_peers.get(&peer).is_some());
		assert_eq!(state.is_peer_available(&peer), false);
	}

	#[test]
	fn ensure_backoff_time() {
		const TEST_BACKOFF_SECONDS: u64 = 2;
		let mut state = DisconnectedPeers {
			disconnected_peers: LruMap::new(ByLength::new(1)),
			backoff_seconds: TEST_BACKOFF_SECONDS,
		};
		let peer = PeerId::random();

		assert!(state.on_disconnect_during_request(peer).is_none());
		assert_eq!(state.is_peer_available(&peer), false);

		// Wait until the backoff time has passed
		std::thread::sleep(Duration::from_secs(TEST_BACKOFF_SECONDS + 1));

		assert_eq!(state.is_peer_available(&peer), true);
	}
}
