//! The screen: the last image the guest showed (a flip of, or a blit to, the
//! primary DirectDraw surface), for the window front end to display.

use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    /// 0x00RRGGBB.
    pub pixels: Vec<u32>,
}

static FRAME: Mutex<Option<(u64, Arc<Frame>)>> = Mutex::new(None);

pub fn publish(frame: Frame) {
    let mut g = FRAME.lock().unwrap();
    let seq = g.as_ref().map(|f| f.0 + 1).unwrap_or(1);
    *g = Some((seq, Arc::new(frame)));
}

/// The frame newer than `seen` (its sequence number), if any.
pub fn newer_than(seen: u64) -> Option<(u64, Arc<Frame>)> {
    let g = FRAME.lock().unwrap();
    match g.as_ref() {
        Some((seq, f)) if *seq > seen => Some((*seq, Arc::clone(f))),
        _ => None,
    }
}
