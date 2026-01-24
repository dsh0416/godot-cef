//! Queue processing utilities for browser-to-Godot communication.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Drains all items from a queue, returning them as a Vec.
pub fn drain_queue<T>(queue: &Arc<Mutex<VecDeque<T>>>) -> Vec<T> {
    match queue.lock() {
        Ok(mut q) => q.drain(..).collect(),
        Err(_) => Vec::new(),
    }
}
