use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::{ThreadId, current},
    time::{Duration, Instant},
};

use anyhow::Context;
use gpui_util::ResultExt;
use windows::{
    System::Threading::{
        ThreadPool, ThreadPoolTimer, TimerElapsedHandler, WorkItemHandler, WorkItemPriority,
    },
    Win32::{
        Foundation::{CloseHandle, LPARAM, WPARAM},
        Media::{timeBeginPeriod, timeEndPeriod},
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentThread,
            INFINITE, SetThreadPriority, SetWaitableTimer, THREAD_PRIORITY_TIME_CRITICAL,
            TIMER_ALL_ACCESS, WaitForSingleObjectEx,
        },
        UI::WindowsAndMessaging::PostMessageW,
    },
    core::PCWSTR,
};

use crate::{HWND, SafeHwnd, WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD};
use gpui::{
    PlatformDispatcher, Priority, PriorityQueueSender, RunnableVariant, TimerResolutionGuard,
};

pub(crate) struct WindowsDispatcher {
    pub(crate) wake_posted: AtomicBool,
    main_sender: PriorityQueueSender<RunnableVariant>,
    main_thread_id: ThreadId,
    pub(crate) platform_window_handle: SafeHwnd,
    validation_number: usize,
}

impl WindowsDispatcher {
    const HIGH_RESOLUTION_TIMER_LIMIT: Duration = Duration::from_millis(20);

    pub(crate) fn new(
        main_sender: PriorityQueueSender<RunnableVariant>,
        platform_window_handle: HWND,
        validation_number: usize,
    ) -> Self {
        let main_thread_id = current().id();
        let platform_window_handle = platform_window_handle.into();

        WindowsDispatcher {
            main_sender,
            main_thread_id,
            platform_window_handle,
            validation_number,
            wake_posted: AtomicBool::new(false),
        }
    }

    fn dispatch_on_threadpool(&self, priority: WorkItemPriority, runnable: RunnableVariant) {
        let handler = {
            let mut task_wrapper = Some(runnable);
            WorkItemHandler::new(move |_| {
                let runnable = task_wrapper.take().unwrap();
                Self::execute_runnable(runnable);
                Ok(())
            })
        };

        ThreadPool::RunWithPriorityAsync(&handler, priority).log_err();
    }

    fn dispatch_on_threadpool_after(&self, runnable: RunnableVariant, duration: Duration) {
        let deadline = Instant::now() + duration;
        if duration <= Self::HIGH_RESOLUTION_TIMER_LIMIT {
            let timer = unsafe {
                CreateWaitableTimerExW(
                    None,
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
            };
            if let Ok(timer) = timer {
                let intervals_100ns =
                    duration.as_nanos().div_ceil(100).clamp(1, i64::MAX as u128) as i64;
                let due_time = -intervals_100ns;
                let armed = unsafe { SetWaitableTimer(timer, &due_time, 0, None, None, false) };
                if armed.is_ok() {
                    let timer_raw = timer.0 as isize;
                    let mut task_wrapper = Some(runnable);
                    let handler = WorkItemHandler::new(move |_| {
                        let timer = windows::Win32::Foundation::HANDLE(timer_raw as *mut _);
                        unsafe {
                            WaitForSingleObjectEx(timer, INFINITE, false);
                            CloseHandle(timer).log_err();
                        }
                        let now = Instant::now();
                        if now > deadline {
                            crate::viewport_bridge::record_viewport_perf(
                                "frame.gpui_timer_wake_lateness",
                                now.duration_since(deadline)
                                    .as_nanos()
                                    .min(u128::from(u64::MAX))
                                    as u64,
                            );
                        }
                        let runnable = task_wrapper.take().unwrap();
                        Self::execute_runnable(runnable);
                        Ok(())
                    });
                    ThreadPool::RunWithPriorityAsync(&handler, WorkItemPriority::High).log_err();
                    return;
                }
                unsafe { CloseHandle(timer).log_err() };
            }
        }
        let handler = {
            let mut task_wrapper = Some(runnable);
            TimerElapsedHandler::new(move |_| {
                let now = Instant::now();
                if now > deadline {
                    crate::viewport_bridge::record_viewport_perf(
                        "frame.gpui_timer_wake_lateness",
                        now.duration_since(deadline)
                            .as_nanos()
                            .min(u128::from(u64::MAX)) as u64,
                    );
                }
                let runnable = task_wrapper.take().unwrap();
                Self::execute_runnable(runnable);
                Ok(())
            })
        };
        ThreadPoolTimer::CreateTimer(&handler, duration.into()).log_err();
    }

    #[inline(always)]
    pub(crate) fn execute_runnable(runnable: RunnableVariant) {
        let location = runnable.metadata().location;
        let spawned = runnable.metadata().spawned;
        gpui::profiler::update_running_task(spawned, location);
        runnable.run();
        gpui::profiler::save_task_timing();
    }
}

impl PlatformDispatcher for WindowsDispatcher {
    fn is_main_thread(&self) -> bool {
        current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        let priority = match priority {
            Priority::RealtimeAudio => {
                panic!("RealtimeAudio priority should use spawn_realtime, not dispatch")
            }
            Priority::High => WorkItemPriority::High,
            Priority::Medium => WorkItemPriority::Normal,
            Priority::Low => WorkItemPriority::Low,
        };
        self.dispatch_on_threadpool(priority, runnable);
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        match self.main_sender.send(priority, runnable) {
            Ok(_) => {
                if !self.wake_posted.swap(true, Ordering::AcqRel) {
                    unsafe {
                        PostMessageW(
                            Some(self.platform_window_handle.as_raw()),
                            WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD,
                            WPARAM(self.validation_number),
                            LPARAM(0),
                        )
                        .log_err();
                    }
                }
            }
            Err(runnable) => {
                // NOTE: Runnable may wrap a Future that is !Send.
                //
                // This is usually safe because we only poll it on the main thread.
                // However if the send fails, we know that:
                // 1. main_receiver has been dropped (which implies the app is shutting down)
                // 2. we are on a background thread.
                // It is not safe to drop something !Send on the wrong thread, and
                // the app will exit soon anyway, so we must forget the runnable.
                std::mem::forget(runnable);
            }
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        self.dispatch_on_threadpool_after(runnable, duration);
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            // SAFETY: always safe to call
            let thread_handle = unsafe { GetCurrentThread() };

            // SAFETY: thread_handle is a valid handle to the current thread
            unsafe { SetThreadPriority(thread_handle, THREAD_PRIORITY_TIME_CRITICAL) }
                .context("thread priority")
                .log_err();

            f();
        });
    }

    fn increase_timer_resolution(&self) -> TimerResolutionGuard {
        unsafe {
            timeBeginPeriod(1);
        }
        gpui_util::defer(Box::new(|| unsafe {
            timeEndPeriod(1);
        }))
    }
}
