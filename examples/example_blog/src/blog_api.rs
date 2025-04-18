use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlogPost {
    pub id: u16,
    pub title: String,
    pub completed: bool,
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use parking_lot::Mutex;
    use std::sync::LazyLock;

    use super::*;

    pub static BLOGPOSTS: LazyLock<Mutex<Vec<BlogPost>>> = LazyLock::new(|| {
        Mutex::new(vec![
            BlogPost {
                id: 1,
                title: "Learn Rust".to_string(),
                completed: false,
            },
            BlogPost {
                id: 2,
                title: "Learn Leptos".to_string(),
                completed: false,
            },
            BlogPost {
                id: 3,
                title: "Build a web app".to_string(),
                completed: false,
            },
        ])
    });
}

#[server]
pub async fn list_blogposts() -> Result<Vec<BlogPost>, ServerFnError> {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Ok(self::ssr::BLOGPOSTS
        .lock()
        .iter()
        .cloned()
        .collect::<Vec<_>>())
}

#[server]
pub async fn add_blogpost(title: String) -> Result<(), ServerFnError> {
    let mut guard = self::ssr::BLOGPOSTS.lock();
    let id = guard.len() + 1;
    guard.push(BlogPost {
        id: id as _,
        title,
        completed: false,
    });
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlogPostFull {
    pub blogpost: BlogPost,
    pub body: String,
}

#[server]
pub async fn get_blogpost_full(id: u16) -> Result<Option<BlogPostFull>, ServerFnError> {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    if let Some(blogpost) = self::ssr::BLOGPOSTS
        .lock()
        .iter()
        .find(|blogpost| blogpost.id == id)
    {
        Ok(Some(BlogPostFull {
            blogpost: blogpost.clone(),
            body: lipsum::lipsum(25),
        }))
    } else {
        Ok(None)
    }
}

#[server]
pub async fn delete_blogpost(id: u16) -> Result<(), ServerFnError> {
    self::ssr::BLOGPOSTS
        .lock()
        .retain(|blogpost| blogpost.id != id);
    Ok(())
}
