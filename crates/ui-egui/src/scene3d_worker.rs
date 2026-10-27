//! Renders the 3D editor's preview off the UI thread, so dragging the camera or a gizmo never
//! waits for a picture. One render runs at a time; while it runs the editor keeps showing the
//! last finished picture, and when it ends the newest wanted state is submitted, so a burst of
//! edits costs one render, not one per edit. The browser build has no threads and renders
//! inline (at the editor's low resolution while the pointer is down).

use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use photocraft_render3d::{Image, RenderOptions};
use photocraft_scene3d::Scene;

/// What a render was asked for: scene revision, width, height, anti-aliasing.
pub type Key = (u64, u32, u32, u32);

type Outcome = Result<Image, String>;

#[derive(Default)]
pub struct PreviewWorker {
    running: Option<(Key, Receiver<Outcome>)>,
    /// A result produced inline (no threads) and not yet taken.
    ready: Option<(Key, Outcome)>,
    /// The key most recently submitted, so a failed render is not retried every frame.
    last: Option<Key>,
}

impl PreviewWorker {
    /// Whether a render is still running.
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// Whether `key` is the last thing submitted (running, finished or failed).
    pub fn is_last(&self, key: Key) -> bool {
        self.last == Some(key)
    }

    /// A finished render, if there is one. Never blocks.
    pub fn poll(&mut self) -> Option<(Key, Outcome)> {
        if let Some(done) = self.ready.take() {
            return Some(done);
        }
        let (key, rx) = self.running.as_ref()?;
        let key = *key;
        match rx.try_recv() {
            Ok(r) => {
                self.running = None;
                Some((key, r))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.running = None;
                Some((key, Err("the preview renderer stopped unexpectedly".into())))
            }
        }
    }

    /// Starts rendering `scene` unless a render is already running.
    pub fn submit(&mut self, ctx: &egui::Context, key: Key, scene: Scene, opts: RenderOptions) {
        if self.busy() {
            return;
        }
        self.last = Some(key);
        let render = move || -> Outcome {
            // A panic in the renderer must not take the editor down with it.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| photocraft_render3d::render(&scene, &opts))) {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err("the preview renderer failed".into()),
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = sync_channel(1);
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("scene3d-preview".into()).spawn(move || {
                let _ = tx.send(render());
                ctx.request_repaint();
            });
            match spawned {
                Ok(_) => self.running = Some((key, rx)),
                Err(e) => self.ready = Some((key, Err(format!("could not start the preview renderer: {e}")))),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, sync_channel::<Outcome>);
            self.ready = Some((key, render()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn wait(w: &mut PreviewWorker) -> (Key, Outcome) {
        let start = Instant::now();
        loop {
            if let Some(done) = w.poll() {
                return done;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "render never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_render_completes_off_thread_and_reports_its_key() {
        let ctx = egui::Context::default();
        let mut w = PreviewWorker::default();
        assert!(w.poll().is_none() && !w.busy());
        w.submit(&ctx, (7, 32, 24, 1), Scene::new(), RenderOptions { ssaa: 1, ..RenderOptions::new(32, 24) });
        let (key, r) = wait(&mut w);
        assert_eq!(key, (7, 32, 24, 1));
        assert_eq!((r.as_ref().unwrap().width, r.unwrap().height), (32, 24));
        assert!(!w.busy() && w.is_last((7, 32, 24, 1)) && !w.is_last((8, 32, 24, 1)));
    }

    #[test]
    fn a_second_submit_while_busy_is_ignored() {
        let ctx = egui::Context::default();
        let mut w = PreviewWorker::default();
        w.submit(&ctx, (1, 600, 600, 2), Scene::new(), RenderOptions::new(600, 600));
        if w.busy() {
            w.submit(&ctx, (2, 8, 8, 1), Scene::new(), RenderOptions::new(8, 8));
            assert!(w.is_last((1, 600, 600, 2)), "the newer request waits for the caller to resubmit");
        }
        assert_eq!(wait(&mut w).0, (1, 600, 600, 2));
    }

    #[test]
    fn a_render_that_cannot_run_reports_an_error_instead_of_a_picture() {
        let ctx = egui::Context::default();
        let mut w = PreviewWorker::default();
        w.submit(&ctx, (1, 0, 0, 1), Scene::new(), RenderOptions::new(0, 0));
        let (_, r) = wait(&mut w);
        assert!(r.unwrap_err().contains("outside"));
    }
}
