//! The frame every dialog shares: the dialog panel on a dimmed backdrop.

use leptos::prelude::*;

/// A dialog panel on a dimmed backdrop. A click on the backdrop closes the dialog with
/// `on_close`, a click in the panel does not. Esc is the App's business: it closes the topmost
/// dialog and leaves a nested one to close itself.
#[component]
pub fn Modal(
    /// Classes of the panel besides `modal`.
    #[prop(optional)]
    class: &'static str,
    /// Classes of the backdrop besides `modal-backdrop`.
    #[prop(optional)]
    backdrop_class: &'static str,
    /// A dialog over another one: a lighter backdrop, whose clicks do not reach the dialog under it.
    #[prop(optional)]
    nested: bool,
    /// What a click on the backdrop does; without it only the dialog's own buttons close it.
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
    /// An alert dialog with this label, for a dialog that needs an answer.
    #[prop(optional)]
    alert: Option<&'static str>,
    children: Children,
) -> impl IntoView {
    let backdrop = ["modal-backdrop", if nested { "nested" } else { "" }, backdrop_class]
        .into_iter()
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let panel = if class.is_empty() {
        "modal".to_owned()
    } else {
        format!("modal {class}")
    };
    view! {
        <div class=backdrop on:click=move |ev| {
            if nested {
                ev.stop_propagation();
            }
            if let Some(close) = on_close {
                close.run(());
            }
        }>
            <div class=panel role=alert.map(|_| "alertdialog") aria-label=alert on:click=|ev| ev.stop_propagation()>
                {children()}
            </div>
        </div>
    }
}
