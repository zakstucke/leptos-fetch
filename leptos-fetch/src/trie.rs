use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

#[derive(Debug)]
struct TrieNode<T> {
    children: HashMap<String, TrieNode<T>>,
    items: HashSet<T>,
}

impl<T> Default for TrieNode<T> {
    fn default() -> Self {
        Self {
            children: HashMap::new(),
            items: HashSet::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Trie<T> {
    root: TrieNode<T>,
}

impl<T> Default for Trie<T> {
    fn default() -> Self {
        Self {
            root: TrieNode::default(),
        }
    }
}

impl<T: Eq + Hash> Trie<T> {
    pub fn insert<S: ToString>(&mut self, path: impl IntoIterator<Item = S>, item: T) {
        let mut node = &mut self.root;
        for segment in path {
            node = node.children.entry(segment.to_string()).or_default();
        }
        node.items.insert(item);
    }

    pub fn find_with_prefix<S: AsRef<str>>(
        &self,
        prefix: impl IntoIterator<Item = S>,
    ) -> HashSet<&T> {
        // Navigate to prefix node
        let mut node = &self.root;
        for segment in prefix {
            match node.children.get(segment.as_ref()) {
                Some(child) => node = child,
                None => return HashSet::new(),
            }
        }

        // Iteratively traverse subtree
        let mut results = HashSet::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            results.extend(&current.items);
            for child in current.children.values() {
                stack.push(child);
            }
        }
        results
    }

    /// Removes the given item from the trie path. Returns true if the item was found and removed.
    pub fn remove<S: AsRef<str>>(&mut self, path: impl IntoIterator<Item = S>, item: &T) -> bool {
        let path: Vec<String> = path.into_iter().map(|s| s.as_ref().to_string()).collect();

        // Traverse to target node
        let mut node = &mut self.root;
        for segment in &path {
            if let Some(child) = node.children.get_mut(segment) {
                node = child;
            } else {
                return false;
            }
        }

        // Remove the item if it exists
        let existed = node.items.remove(item);

        // Gc pathway if needed:
        if existed && node.items.is_empty() && node.children.is_empty() {
            // Go from deepest to root:
            for depth in (0..path.len()).rev() {
                let mut current = &mut self.root;

                for segment in &path[..depth] {
                    current = current
                        .children
                        .get_mut(segment)
                        .expect("Path should exist");
                }

                let segment = &path[depth];
                if let Some(child) = current.children.get(segment) {
                    if child.items.is_empty() && child.children.is_empty() {
                        current.children.remove(segment);
                    } else {
                        break; // Stop on a non-empty node
                    }
                }
            }
        }

        existed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trie_find_exact_and_longer_paths() {
        let mut trie = Trie::default();
        trie.insert(["a", "b"], 1);
        trie.insert(["a", "b", "c"], 2);
        trie.insert(["a", "b", "c", "d"], 3);
        trie.insert(["x", "y"], 4);

        let results = trie.find_with_prefix(["a", "b"]);
        assert!(results.contains(&&1));
        assert!(results.contains(&&2));
        assert!(results.contains(&&3));
        assert_eq!(results.len(), 3);

        let results = trie.find_with_prefix(["a", "b", "c"]);
        assert!(results.contains(&&2));
        assert!(results.contains(&&3));
        assert_eq!(results.len(), 2);

        let results = trie.find_with_prefix(["a"]);
        assert!(results.contains(&&1));
        assert!(results.contains(&&2));
        assert!(results.contains(&&3));
        assert_eq!(results.len(), 3);

        let results = trie.find_with_prefix(["a", "b", "c", "d", "e"]);
        assert!(results.is_empty());

        let results = trie.find_with_prefix(["x"]);
        assert!(results.contains(&&4));
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_trie_find_with_empty_path_matches_all() {
        let mut trie = Trie::default();
        trie.insert(["a"], 1);
        trie.insert(["b"], 2);
        trie.insert(["a", "c"], 3);

        let results = trie.find_with_prefix(Vec::<&str>::new());
        assert_eq!(results.len(), 3);
        assert!(results.contains(&&1));
        assert!(results.contains(&&2));
        assert!(results.contains(&&3));
    }

    #[test]
    fn test_trie_remove_single_item() {
        let mut trie = Trie::default();
        trie.insert(["a", "b"], 1);
        trie.insert(["a", "b"], 2);

        assert!(trie.remove(["a", "b"], &1));

        let results = trie.find_with_prefix(["a", "b"]);
        assert_eq!(results.len(), 1);
        assert_eq!(results.iter().next(), Some(&&2));
    }

    #[test]
    fn test_trie_remove_only_specific_path() {
        let mut trie = Trie::default();
        trie.insert(["a", "b"], 1);
        trie.insert(["a", "b", "c"], 1);

        assert!(trie.remove(["a", "b", "c"], &1));

        let results = trie.find_with_prefix(["a", "b"]);
        assert!(results.contains(&&1));
        assert_eq!(results.len(), 1);

        let results = trie.find_with_prefix(["a", "b", "c"]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_trie_remove_item_does_not_remove_others() {
        let mut trie = Trie::default();
        trie.insert(["x", "y"], 5);
        trie.insert(["x", "y"], 6);

        assert!(trie.remove(["x", "y"], &5));

        let results = trie.find_with_prefix(["x", "y"]);
        assert_eq!(results.iter().next(), Some(&&6));
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_trie_gc_path_after_removal() {
        let mut trie = Trie::default();
        trie.insert(["root", "leaf1"], 1);
        trie.insert(["root", "leaf2"], 2);

        assert!(trie.remove(["root", "leaf1"], &1));

        // Still another leaf, so root should remain
        let results = trie.find_with_prefix(["root"]);
        assert!(results.contains(&&2));
        assert_eq!(results.len(), 1);
        assert!(trie.root.children.contains_key("root"));

        assert!(trie.remove(["root", "leaf2"], &2));

        // Try to re-traverse that path; it should not exist
        let results = trie.find_with_prefix(["root"]);
        assert!(results.is_empty());
        assert!(!trie.root.children.contains_key("root"));
    }
}
