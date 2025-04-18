use super::*;
use leptos::prelude::*;

#[component]
pub fn Button(children: Children, color: ColorOption) -> impl IntoView {
    view! {
        <button
            type="button"
            class=match color {
                ColorOption::Blue => {
                    "lq-text-white lq-bg-blue-700 lq-hover:bg-blue-800 lq-focus:outline-none lq-focus:ring-4 lq-focus:ring-blue-300 lq-font-medium lq-rounded-full lq-text-xs lq-px-2 lq-py-1 lq-text-center lq-dark:bg-blue-600 lq-dark:hover:bg-blue-700 lq-dark:focus:ring-blue-800"
                }
                ColorOption::Gray => {
                    "lq-text-gray-900 lq-bg-white lq-border lq-border-gray-300 lq-focus:outline-none lq-hover:bg-gray-100 lq-focus:ring-4 lq-focus:ring-gray-200 lq-font-medium lq-rounded-full lq-text-xs lq-px-2 lq-py-1 lq-dark:bg-gray-800 lq-dark:text-white lq-dark:border-gray-600 lq-dark:hover:bg-gray-700 lq-dark:hover:border-gray-600 lq-dark:focus:ring-gray-700"
                }
                ColorOption::Green => {
                    "lq-text-white lq-bg-green-700 lq-hover:bg-green-800 lq-focus:outline-none lq-focus:ring-4 lq-focus:ring-green-300 lq-font-medium lq-rounded-full lq-text-xs lq-px-2 lq-py-1 lq-text-center lq-dark:bg-green-600 lq-dark:hover:bg-green-700 lq-dark:focus:ring-green-800"
                }
                ColorOption::Red => {
                    "lq-text-white lq-bg-red-700 lq-hover:bg-red-800 lq-focus:outline-none lq-focus:ring-4 lq-focus:ring-red-300 lq-font-medium lq-rounded-full lq-text-xs lq-px-2 lq-py-1 lq-text-center lq-dark:bg-red-600 lq-dark:hover:bg-red-700 lq-dark:focus:ring-red-900"
                }
                ColorOption::Yellow => {
                    "lq-text-white lq-bg-yellow-400 lq-hover:bg-yellow-500 lq-focus:outline-none lq-focus:ring-4 lq-focus:ring-yellow-300 lq-font-medium lq-rounded-full lq-text-xs lq-px-2 lq-py-1 lq-text-center lq-dark:focus:ring-yellow-900"
                }
            }
        >
            {children()}
        </button>
    }
}
