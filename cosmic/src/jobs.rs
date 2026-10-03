//! One worker and one latest repeat per data source. Worker ownership survives cancelled awaits.
use cosmic::app::Task;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Kind {
    Metadata,
    Summary,
    Operation,
    #[cfg(test)]
    Vpn,
    #[cfg(test)]
    AurSearch,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata | Self::Operation => cm::t!("Состояние"),
            Self::Summary => cm::t!("Обновления"),
            #[cfg(test)]
            Self::Vpn => "VPN",
            #[cfg(test)]
            Self::AurSearch => "AUR",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum Phase {
    #[default]
    Idle,
    Loading,
    Ready,
    Error(String),
}
type Work<M> = Box<dyn FnOnce() -> Result<M, String> + Send>;
struct Slot<M> {
    phase: Phase,
    key: String,
    generation: u64,
    flight: u64,
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    latest: Option<Work<M>>,
    retry_at: Option<Instant>,
    failures: u32,
}
impl<M> Default for Slot<M> {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            key: String::new(),
            generation: 0,
            flight: 0,
            busy: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            latest: None,
            retry_at: None,
            failures: 0,
        }
    }
}
pub struct Jobs<M> {
    slots: BTreeMap<Kind, Slot<M>>,
}
impl<M> Default for Jobs<M> {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Completion<M> {
    pub kind: Kind,
    generation: u64,
    flight: u64,
    pub result: Result<Box<M>, String>,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct WorkerLease(Arc<AtomicBool>);
impl Drop for WorkerLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl<M: Send + 'static> Jobs<M> {
    pub fn request(
        &mut self,
        kind: Kind,
        key: String,
        work: impl FnOnce() -> Result<M, String> + Send + 'static,
        message: fn(Completion<M>) -> M,
    ) -> Task<M> {
        let slot = self.slots.entry(kind).or_default();
        if slot.key != key {
            slot.key = key;
            slot.generation += 1;
            slot.cancel.store(true, Ordering::Release);
            slot.latest = None;
            slot.retry_at = None;
            slot.failures = 0;
        }
        if slot.retry_at.is_some_and(|at| at > Instant::now()) {
            return Task::none();
        }
        slot.latest = Some(Box::new(work));
        Self::launch(kind, slot, message)
    }
    fn launch(kind: Kind, slot: &mut Slot<M>, message: fn(Completion<M>) -> M) -> Task<M> {
        if slot.busy.load(Ordering::Acquire) {
            return Task::none();
        }
        let Some(work) = slot.latest.take() else {
            return Task::none();
        };
        slot.busy.store(true, Ordering::Release);
        slot.cancel = Arc::new(AtomicBool::new(false));
        slot.phase = Phase::Loading;
        slot.flight += 1;
        let (generation, flight) = (slot.generation, slot.flight);
        let busy = slot.busy.clone();
        let cancel = slot.cancel.clone();
        let guard = CancelOnDrop(cancel.clone());
        // Construct the lease before scheduling: dropping an unpolled task also releases its slot.
        let lease = WorkerLease(busy);
        cosmic::task::future(async move {
            let _guard = guard;
            let result = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                cm::common::with_probe_scope(Duration::from_secs(30), cancel, work)
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .map(Box::new)
            })
            .await
            .unwrap_or_else(|e| Err(format!("background worker failed: {e}")));
            message(Completion {
                kind,
                generation,
                flight,
                result,
            })
        })
    }
    pub fn complete(
        &mut self,
        completion: &Completion<M>,
        message: fn(Completion<M>) -> M,
    ) -> (bool, Task<M>) {
        let Some(slot) = self.slots.get_mut(&completion.kind) else {
            return (false, Task::none());
        };
        let accept = completion.generation == slot.generation && completion.flight == slot.flight;
        if completion.flight != slot.flight {
            return (false, Task::none());
        }
        if accept {
            match &completion.result {
                Ok(_) => {
                    slot.phase = Phase::Ready;
                    slot.failures = 0;
                    slot.retry_at = None;
                }
                Err(e) => {
                    slot.phase = Phase::Error(e.clone());
                    slot.failures = slot.failures.saturating_add(1);
                    slot.retry_at =
                        Some(Instant::now() + Duration::from_secs(1 << slot.failures.min(5)));
                    slot.latest = None;
                }
            }
        }
        (accept, Self::launch(completion.kind, slot, message))
    }
    #[cfg(test)]
    pub fn invalidate(&mut self, kinds: &[Kind]) {
        for kind in kinds {
            if let Some(slot) = self.slots.get_mut(kind) {
                slot.generation += 1;
                slot.key.clear();
                slot.latest = None;
                slot.cancel.store(true, Ordering::Release);
                slot.phase = Phase::Idle;
                slot.retry_at = None;
                slot.failures = 0;
            }
        }
    }
    #[cfg(test)]
    pub fn demo_phase(&mut self, kind: Kind, phase: Phase) {
        self.slots.entry(kind).or_default().phase = phase;
    }
    pub fn phase(&self, kind: Kind) -> Phase {
        self.slots
            .get(&kind)
            .map(|s| s.phase.clone())
            .unwrap_or_default()
    }
    pub fn errors(&self) -> impl Iterator<Item = (Kind, &str)> {
        self.slots.iter().filter_map(|(k, s)| match &s.phase {
            Phase::Error(e) => Some((*k, e.as_str())),
            _ => None,
        })
    }
}
impl<M> Drop for Jobs<M> {
    fn drop(&mut self) {
        for s in self.slots.values() {
            s.cancel.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::futures::StreamExt;
    use std::sync::mpsc;
    #[derive(Clone, Debug)]
    enum Message {
        Done(Completion<Message>),
        Value(u32),
    }
    async fn completion(task: Task<Message>) -> Completion<Message> {
        let mut stream = cosmic::iced::runtime::task::into_stream(task).unwrap();
        match stream.next().await.unwrap() {
            cosmic::iced::runtime::Action::Output(cosmic::Action::App(Message::Done(c))) => c,
            _ => panic!("unexpected action"),
        }
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }
    #[test]
    fn hundred_ticks_queue_only_latest_and_keep_heartbeat_alive() {
        runtime().block_on(async {
            let mut jobs = Jobs::default();
            let (release, blocked) = mpsc::channel();
            let (entered, started) = mpsc::channel();
            let task = jobs.request(
                Kind::Summary,
                "same".into(),
                move || {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(Message::Value(0))
                },
                Message::Done,
            );
            let handle = tokio::spawn(completion(task));
            while started.try_recv().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            for value in 1..=100 {
                let count = count.clone();
                let task = jobs.request(
                    Kind::Summary,
                    "same".into(),
                    move || {
                        count.fetch_add(1, Ordering::Relaxed);
                        Ok(Message::Value(value))
                    },
                    Message::Done,
                );
                assert!(cosmic::iced::runtime::task::into_stream(task).is_none());
            }
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(count.load(Ordering::Relaxed), 0);
            // A real thirty-second stalled probe must not admit overlapping workers.
            tokio::time::sleep(Duration::from_secs(30)).await;
            release.send(()).unwrap();
            let first = handle.await.unwrap();
            let (accept, repeat) = jobs.complete(&first, Message::Done);
            assert!(accept);
            assert!(
                matches!(jobs.phase(Kind::Summary), Phase::Error(e) if e.contains("timed out"))
            );
            assert!(cosmic::iced::runtime::task::into_stream(repeat).is_none());
            assert_eq!(count.load(Ordering::Relaxed), 0);
            jobs.slots.get_mut(&Kind::Summary).unwrap().retry_at = None;
            let retry_count = count.clone();
            let retry = jobs.request(
                Kind::Summary,
                "same".into(),
                move || {
                    retry_count.fetch_add(1, Ordering::Relaxed);
                    Ok(Message::Value(100))
                },
                Message::Done,
            );
            assert!(matches!(
                completion(retry).await.result.as_deref(),
                Ok(Message::Value(100))
            ));
            assert_eq!(count.load(Ordering::Relaxed), 1);
        });
    }
    #[test]
    fn old_query_and_closed_page_results_are_rejected() {
        runtime().block_on(async {
            let mut jobs = Jobs::default();
            let (release, blocked) = mpsc::channel();
            let (entered, started) = mpsc::channel();
            let a = jobs.request(
                Kind::AurSearch,
                "A".into(),
                move || {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(Message::Value(1))
                },
                Message::Done,
            );
            let handle = tokio::spawn(completion(a));
            while started.try_recv().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let b = jobs.request(
                Kind::AurSearch,
                "B".into(),
                || Ok(Message::Value(2)),
                Message::Done,
            );
            assert!(cosmic::iced::runtime::task::into_stream(b).is_none());
            release.send(()).unwrap();
            let a = handle.await.unwrap();
            let (accept, b) = jobs.complete(&a, Message::Done);
            assert!(!accept);
            let b = completion(b).await;
            assert!(matches!(b.result.as_deref(), Ok(Message::Value(2))));
            jobs.invalidate(&[Kind::AurSearch]);
            let (accept, repeat) = jobs.complete(&b, Message::Done);
            assert!(!accept);
            assert!(cosmic::iced::runtime::task::into_stream(repeat).is_none());
            assert!(matches!(jobs.phase(Kind::AurSearch), Phase::Idle));
        });
    }
    #[test]
    fn cancelled_await_does_not_release_running_worker() {
        runtime().block_on(async {
            let mut jobs = Jobs::default();
            let (release, blocked) = mpsc::channel();
            let (entered, started) = mpsc::channel();
            let task = jobs.request(
                Kind::Vpn,
                "vpn".into(),
                move || {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(Message::Value(1))
                },
                Message::Done,
            );
            let handle = tokio::spawn(completion(task));
            while started.try_recv().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            handle.abort();
            let _ = handle.await;
            let slot = jobs.slots.get(&Kind::Vpn).unwrap();
            assert!(slot.cancel.load(Ordering::Acquire));
            assert!(slot.busy.load(Ordering::Acquire));
            let retry = jobs.request(
                Kind::Vpn,
                "vpn".into(),
                || Ok(Message::Value(2)),
                Message::Done,
            );
            assert!(cosmic::iced::runtime::task::into_stream(retry).is_none());
            release.send(()).unwrap();
            while jobs.slots[&Kind::Vpn].busy.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let retry = jobs.request(
                Kind::Vpn,
                "vpn".into(),
                || Ok(Message::Value(3)),
                Message::Done,
            );
            assert!(matches!(
                completion(retry).await.result.as_deref(),
                Ok(Message::Value(3))
            ));
        });
    }
    #[test]
    fn failed_worker_is_error_and_retry_has_backoff() {
        runtime().block_on(async {
            let mut jobs = Jobs::default();
            let task = jobs.request(
                Kind::Summary,
                "".into(),
                || Err("probe failed".into()),
                Message::Done,
            );
            let c = completion(task).await;
            assert!(jobs.complete(&c, Message::Done).0);
            assert!(matches!(jobs.phase(Kind::Summary), Phase::Error(e) if e == "probe failed"));
            let retry = jobs.request(
                Kind::Summary,
                "".into(),
                || Ok(Message::Value(1)),
                Message::Done,
            );
            assert!(cosmic::iced::runtime::task::into_stream(retry).is_none());
        });
    }
}
