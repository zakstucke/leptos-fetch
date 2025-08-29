use futures::Future;
use leptos::prelude::untrack;
use pin_project_lite::pin_project;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pin_project! {
    #[derive(Clone)]
    #[allow(missing_docs)]
    pub(crate) struct NoReactiveDiagnosticsFuture<Fut> {
        #[pin]
        pub fut: Fut,
    }
}

impl<Fut> NoReactiveDiagnosticsFuture<Fut> {
    pub fn new(fut: Fut) -> Self {
        Self { fut }
    }
}

impl<Fut: Future> Future for NoReactiveDiagnosticsFuture<Fut> {
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        untrack(|| this.fut.poll(cx))
    }
}
