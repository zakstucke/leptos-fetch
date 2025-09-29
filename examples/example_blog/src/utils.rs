use std::time::Duration;

#[allow(unused_variables)]
pub async fn sleep_compat(duration: Duration) {
    #[cfg(target_arch = "wasm32")]
    send_wrapper::SendWrapper::new(gloo_timers::future::TimeoutFuture::new(
        duration.as_millis().min(u32::MAX as _) as u32,
    ))
    .await;
    #[cfg(feature = "ssr")]
    tokio::time::sleep(duration).await;
}
