// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Runtime abstraction over task spawning and monotonic clocks, so the crate
//! runs on native Tokio hosts and on single-threaded wasm32 hosts (browsers,
//! Cloudflare Workers) alike.

use std::{future::Future, time::Duration};

use futures::future::{AbortHandle, Abortable};

/// Monotonic instant. On wasm32 `std::time::Instant::now()` aborts, so a
/// JS-clock-backed drop-in replacement is used there.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) use std::time::Instant;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use web_time::Instant;

/// Wall clock. `std::time::SystemTime::now()` also aborts on wasm32.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use web_time::{SystemTime, UNIX_EPOCH};

/// Spawns a future on the host runtime and returns a handle that aborts it.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn spawn_abortable<F>(future: F) -> AbortHandle
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (handle, registration) = AbortHandle::new_pair();
    drop(tokio::spawn(async move {
        let _ = Abortable::new(future, registration).await;
    }));
    handle
}

/// Spawns a future on the JS microtask queue and returns a handle that aborts it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn spawn_abortable<F>(future: F) -> AbortHandle
where
    F: std::future::Future<Output = ()> + 'static,
{
    let (handle, registration) = AbortHandle::new_pair();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = Abortable::new(future, registration).await;
    });
    handle
}

/// Returned by [`timeout`] when the deadline passes before the future completes.
#[derive(Debug)]
pub(crate) struct Elapsed;

/// Runs `future` until it completes or `duration` passes.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, Elapsed> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| Elapsed)
}

/// Runs `future` until it completes or a JS `setTimeout` timer for `duration` fires.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, Elapsed> {
    use futures::future::{Either, select};
    match select(std::pin::pin!(future), Delay::new(duration)).await {
        Either::Left((output, _)) => Ok(output),
        Either::Right(_) => Err(Elapsed),
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
struct Delay(wasm_bindgen_futures::JsFuture);

// SAFETY: wasm32-unknown-unknown hosts run on one thread, so the JS promise never
// moves to another thread.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
unsafe impl Send for Delay {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Delay {
    fn new(duration: Duration) -> Self {
        let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let _ = set_timeout(&resolve, millis);
        });
        Self(wasm_bindgen_futures::JsFuture::from(promise))
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Future for Delay {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        std::pin::Pin::new(&mut self.0).poll(cx).map(|_| ())
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(handler: &js_sys::Function, millis: i32) -> wasm_bindgen::JsValue;
}
