use std::{
    cmp::Ordering as CmpOrdering,
    collections::BinaryHeap,
    mem::MaybeUninit,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use gpui::{
    PlatformDispatcher, Priority, PriorityQueueReceiver, PriorityQueueSender, RunnableVariant,
    profiler,
};
use parking_lot::Mutex;

const MINIMUM_WORKER_THREADS: usize = 2;

pub(crate) struct OhosDispatcher {
    background_sender: Arc<PriorityQueueSender<RunnableVariant>>,
    main_sender: Arc<PriorityQueueSender<RunnableVariant>>,
    main_receiver: Mutex<PriorityQueueReceiver<RunnableVariant>>,
    timer_sender: mpsc::Sender<TimerEntry>,
    main_thread_id: thread::ThreadId,
    wake: Arc<MainThreadWake>,
    main_queue_draining: AtomicBool,
}

struct MainThreadWake {
    callback: Mutex<Option<Arc<dyn Fn() -> Result<()> + Send + Sync>>>,
    request_outstanding: AtomicBool,
}

struct TimerEntry {
    due: Instant,
    sequence: u64,
    runnable: RunnableVariant,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.due == other.due && self.sequence == other.sequence
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .due
            .cmp(&self.due)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl OhosDispatcher {
    pub(crate) fn new() -> Result<Arc<Self>> {
        let (background_sender, background_receiver) = PriorityQueueReceiver::new();
        let (main_sender, main_receiver) = PriorityQueueReceiver::new();
        let background_sender = Arc::new(background_sender);
        let main_sender = Arc::new(main_sender);
        let wake = Arc::new(MainThreadWake {
            callback: Mutex::new(None),
            request_outstanding: AtomicBool::new(false),
        });

        let thread_count = thread::available_parallelism()
            .map_or(MINIMUM_WORKER_THREADS, |count| {
                count.get().max(MINIMUM_WORKER_THREADS)
            });
        for index in 0..thread_count {
            let receiver = background_receiver.clone();
            thread::Builder::new()
                .name(format!("GPUI-Worker-{index}"))
                .spawn(move || run_background_worker(receiver))
                .with_context(|| format!("failed to spawn GPUI worker {index}"))?;
        }
        drop(background_receiver);

        let (timer_sender, timer_receiver) = mpsc::channel();
        thread::Builder::new()
            .name("GPUI-Timer".to_owned())
            .spawn(move || run_timer_thread(timer_receiver))
            .context("failed to spawn GPUI timer thread")?;

        Ok(Arc::new(Self {
            background_sender,
            main_sender,
            main_receiver: Mutex::new(main_receiver),
            timer_sender,
            main_thread_id: thread::current().id(),
            wake,
            main_queue_draining: AtomicBool::new(false),
        }))
    }

    pub(crate) fn install_wake_callback(
        &self,
        callback: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) {
        *self.wake.callback.lock() = Some(callback);
    }

    pub(crate) fn clear_wake_callback(&self) {
        self.wake.callback.lock().take();
        self.wake
            .request_outstanding
            .store(false, Ordering::Release);
    }

    pub(crate) fn request_main_wake(&self) {
        self.wake.request();
    }

    pub(crate) fn begin_frame(&self) {
        self.wake
            .request_outstanding
            .store(false, Ordering::Release);
    }

    pub(crate) fn drain_main_queue(&self) {
        // Opening a GPUI window can synchronously cause ArkUI to deliver the
        // already-created XComponent surface. That native callback re-enters
        // the platform while the foreground runnable still holds GPUI's app
        // RefCell. Leave the nested drain to the outer loop so another queued
        // runnable cannot attempt a second mutable app borrow.
        if self.main_queue_draining.swap(true, Ordering::AcqRel) {
            return;
        }
        struct DrainGuard<'a>(&'a AtomicBool);
        impl Drop for DrainGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _drain_guard = DrainGuard(&self.main_queue_draining);

        loop {
            let runnable = {
                let mut receiver = self.main_receiver.lock();
                match receiver.try_pop() {
                    Ok(Some(runnable)) => Some(runnable),
                    Ok(None) | Err(_) => None,
                }
            };
            let Some(runnable) = runnable else {
                break;
            };

            // Never hold the receiver lock while user code runs.
            run_runnable(runnable);
        }
    }
}

impl MainThreadWake {
    fn request(self: &Arc<Self>) {
        let Some(callback) = self.callback.lock().clone() else {
            return;
        };
        if self.request_outstanding.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Err(error) = callback() {
            self.request_outstanding.store(false, Ordering::Release);
            crate::log_message(
                crate::LogLevel::Error,
                format!("failed to schedule an ArkUI frame: {error:#}"),
            );
        }
    }
}

impl PlatformDispatcher for OhosDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        if let Err(error) = self.background_sender.send(priority, runnable) {
            std::mem::forget(error.0);
        }
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        if let Err(error) = self.main_sender.send(priority, runnable) {
            std::mem::forget(error.0);
            return;
        }
        self.request_main_wake();
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        static NEXT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let entry = TimerEntry {
            due: Instant::now() + duration,
            sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            runnable,
        };
        if let Err(error) = self.timer_sender.send(entry) {
            std::mem::forget(error.0.runnable);
        }
    }

    fn spawn_realtime(&self, callback: Box<dyn FnOnce() + Send>) {
        let result = thread::Builder::new()
            .name("GPUI-Realtime".to_owned())
            .spawn(move || {
                // SAFETY: pthread_self always returns the calling thread.
                let thread_id = unsafe { libc::pthread_self() };
                // SAFETY: A zeroed sched_param is valid and fully initialized.
                let mut parameters =
                    unsafe { MaybeUninit::<libc::sched_param>::zeroed().assume_init() };
                parameters.sched_priority = 65;
                // SAFETY: The thread and parameter pointers are live for this call.
                let status = unsafe {
                    libc::pthread_setschedparam(thread_id, libc::SCHED_FIFO, &parameters)
                };
                if status != 0 {
                    crate::log_message(
                        crate::LogLevel::Warning,
                        format!("failed to enable realtime scheduling: {status}"),
                    );
                }
                callback();
            });
        if let Err(error) = result {
            crate::log_message(
                crate::LogLevel::Error,
                format!("failed to spawn realtime thread: {error}"),
            );
        }
    }
}

fn run_background_worker(receiver: PriorityQueueReceiver<RunnableVariant>) {
    for runnable in receiver.iter() {
        run_runnable(runnable);
    }
}

fn run_timer_thread(receiver: mpsc::Receiver<TimerEntry>) {
    let mut timers: BinaryHeap<TimerEntry> = BinaryHeap::new();
    loop {
        let received = match timers.peek() {
            Some(next) => {
                let timeout = next.due.saturating_duration_since(Instant::now());
                receiver.recv_timeout(timeout).ok()
            }
            None => match receiver.recv() {
                Ok(entry) => Some(entry),
                Err(_) => return,
            },
        };

        if let Some(entry) = received {
            timers.push(entry);
        }
        while timers
            .peek()
            .is_some_and(|entry| entry.due <= Instant::now())
        {
            if let Some(entry) = timers.pop() {
                run_runnable(entry.runnable);
            }
        }
    }
}

fn run_runnable(runnable: RunnableVariant) {
    let location = runnable.metadata().location;
    let spawned = runnable.metadata().spawned;
    profiler::update_running_task(spawned, location);
    runnable.run();
    profiler::save_task_timing();
}
