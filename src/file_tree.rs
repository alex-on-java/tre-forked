use slab::Slab;
use indexmap::IndexMap;
use std::fs::{self, Metadata};
use std::path::{Component, Path, PathBuf};

fn to_string(component: &Component) -> String {
    (*component).as_os_str().to_string_lossy().into_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    File,
    Directory,
    Link,
}

impl FileType {
    pub fn new(meta: Metadata) -> FileType {
        let t = meta.file_type();
        if t.is_dir() {
            FileType::Directory
        } else if t.is_symlink() {
            FileType::Link
        } else {
            FileType::File
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypeSpecficData {
    File,
    Directory(IndexMap<String, usize>),
    Link(String),
}

#[derive(Debug, Clone)]
pub struct File {
    pub id: usize,
    parent: Option<usize>,
    pub display_name: String,
    pub path: String,
    pub file_type: FileType,
    pub data: TypeSpecficData,

    // Metadata for smart line-limited display (used by --lines flag)
    #[allow(dead_code)]
    /// Total count of all descendants (files + directories) below this node
    pub total_descendants: usize,
    #[allow(dead_code)]
    /// Number of direct child directories
    pub child_dir_count: usize,
    #[allow(dead_code)]
    /// Number of direct child files (non-directories)
    pub child_file_count: usize,
}

impl File {
    pub fn children_count(&self) -> usize {
        if let TypeSpecficData::Directory(children) = &self.data {
            children.len()
        } else {
            0
        }
    }

    pub fn children(&self) -> Option<&IndexMap<String, usize>> {
        if let TypeSpecficData::Directory(children) = &self.data {
            Some(children)
        } else {
            None
        }
    }

    pub fn link(&self) -> Option<String> {
        if let TypeSpecficData::Link(link) = &self.data {
            Some(link.clone())
        } else {
            None
        }
    }

    /// Returns true if this directory has exactly one child directory and no files.
    /// Used to detect "skinny" paths like `src/main/java/de/konux/` that can be collapsed.
    #[allow(dead_code)]
    pub fn is_skinny(&self) -> bool {
        self.child_dir_count == 1 && self.child_file_count == 0
    }

    fn child_key(&self, name: &str) -> Option<usize> {
        if let TypeSpecficData::Directory(children) = &self.data {
            children.get(name).cloned()
        } else {
            None
        }
    }

    fn add_child(&mut self, name: &str, id: usize) {
        if let TypeSpecficData::Directory(children) = &mut self.data {
            children.insert(name.to_string(), id);
        }
    }

    #[cfg(test)]
    fn is_file(&self) -> bool {
        if let TypeSpecficData::File = self.data {
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    fn is_dir(&self) -> bool {
        if let TypeSpecficData::Directory(_) = self.data {
            true
        } else {
            false
        }
    }
}

pub struct FileTree {
    pub storage: Slab<Box<File>>,
    pub root_id: usize,
}

impl FileTree {
    pub fn new(root_path: &str, children: Vec<(String, FileType)>) -> Option<FileTree> {
        let mut slab = Slab::new();
        let root_entry = slab.vacant_entry();
        let root_id = root_entry.key();

        let root_prefix_len: usize = Path::new(root_path)
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .count();

        let root = Box::new(File {
            id: root_id,
            parent: None,
            display_name: root_path.to_string(),
            path: root_path.to_string(),
            file_type: FileType::Directory,
            data: TypeSpecficData::Directory(IndexMap::new()),
            total_descendants: 0,
            child_dir_count: 0,
            child_file_count: 0,
        });
        root_entry.insert(root);

        for (path, meta) in children {
            let data_option: Option<TypeSpecficData> = match meta {
                FileType::Link => fs::read_link(&path)
                    .ok()
                    .and_then(|path| path.to_str().map(|x| x.to_string()))
                    .map(TypeSpecficData::Link),
                FileType::Directory => Some(TypeSpecficData::Directory(IndexMap::new())),
                FileType::File => Some(TypeSpecficData::File),
            };

            let data = data_option.unwrap();

            let mut ancestry: Vec<Component> = Path::new(&path)
                .components()
                .filter(|c| !matches!(c, Component::CurDir))
                .skip(root_prefix_len)
                .collect();

            let ancestor = ancestry.pop().map(|x| to_string(&x));

            if ancestor.is_none() {
                continue;
            }

            let path_name = ancestor.unwrap();

            // Handle intermidiary directories.
            let mut current_acestor_id = root_id;
            let mut current_ancestor_path = PathBuf::new();
            current_ancestor_path.push(root_path);
            for ancestor_name in ancestry {
                current_ancestor_path.push(ancestor_name);
                let display_name = to_string(&ancestor_name);
                if let Some(child_key) = slab[current_acestor_id].child_key(&display_name) {
                    current_acestor_id = child_key;
                } else {
                    let new_entry = slab.vacant_entry();
                    let new_id = new_entry.key();
                    new_entry.insert(Box::new(File {
                        id: new_id,
                        parent: Some(current_acestor_id),
                        display_name: display_name.clone(),
                        path: current_ancestor_path.to_string_lossy().into_owned(),
                        file_type: FileType::Directory,
                        data: TypeSpecficData::Directory(IndexMap::new()),
                        total_descendants: 0,
                        child_dir_count: 0,
                        child_file_count: 0,
                    }));
                    slab[current_acestor_id].add_child(&display_name, new_id);
                    current_acestor_id = new_id;
                }
            }

            // Finally, insert the node.
            let new_entry = slab.vacant_entry();
            let new_id = new_entry.key();
            new_entry.insert(Box::new(File {
                id: new_id,
                parent: Some(current_acestor_id),
                display_name: path_name.clone(),
                path,
                file_type: meta,
                data,
                total_descendants: 0,
                child_dir_count: 0,
                child_file_count: 0,
            }));
            slab[current_acestor_id].add_child(&path_name, new_id);
        }

        Some(FileTree {
            storage: slab,
            root_id,
        })
    }

    /// Recursively compute metadata for all nodes.
    /// Must be called after tree construction and before using metadata fields.
    #[allow(dead_code)]
    pub fn compute_metadata(&mut self) {
        self.compute_metadata_recursive(self.root_id);
    }

    /// Post-order traversal: compute children first, then aggregate into parent.
    #[allow(dead_code)]
    fn compute_metadata_recursive(&mut self, node_id: usize) -> usize {
        let children: Vec<usize> = {
            let node = &self.storage[node_id];
            if let TypeSpecficData::Directory(children_map) = &node.data {
                children_map.values().cloned().collect()
            } else {
                vec![]
            }
        };

        let mut total_descendants: usize = 0;
        let mut child_dir_count: usize = 0;
        let mut child_file_count: usize = 0;

        for child_id in children {
            // Recursively compute child's metadata first
            let child_descendants = self.compute_metadata_recursive(child_id);
            total_descendants += 1 + child_descendants;

            let child_type = self.storage[child_id].file_type.clone();
            match child_type {
                FileType::Directory => child_dir_count += 1,
                FileType::File | FileType::Link => child_file_count += 1,
            }
        }

        // Update this node's metadata
        let node = &mut self.storage[node_id];
        node.total_descendants = total_descendants;
        node.child_dir_count = child_dir_count;
        node.child_file_count = child_file_count;

        total_descendants
    }

    pub fn get(&self, id: usize) -> &File {
        &self.storage[id]
    }

    pub fn get_root(&self) -> &File {
        self.get(self.root_id)
    }

    pub fn get_parent(&self, file: &File) -> Option<&File> {
        file.parent.map(|id| self.get(id))
    }
}

#[cfg(test)]
mod test {
    use super::{FileTree, FileType, TypeSpecficData};
    use std::path::Path;

    #[test]
    fn tree_construction() {
        let tree = FileTree::new(
            ".",
            vec![
                ("a".to_string(), FileType::File),
                ("b/c/d".to_string(), FileType::File),
            ],
        )
        .unwrap();

        let root = tree.get(tree.root_id);
        assert!(root.is_dir());
        if let TypeSpecficData::Directory(root_chilren) = &root.data {
            assert_eq!(root_chilren.len(), 2);
            let a_id = root_chilren.get("a").expect("a exists");
            let a = tree.get(*a_id);
            assert!(a.is_file());
            assert_eq!(Path::new(&a.path), Path::new("a"));
            let b_id = root_chilren.get("b").expect("b exists");
            let b = tree.get(*b_id);
            assert!(b.is_dir());
            assert_eq!(Path::new(&b.path), Path::new("./b"));
            if let TypeSpecficData::Directory(b_children) = &b.data {
                assert_eq!(b_children.len(), 1);
                let c_id = b_children.get("c").expect("c exists");
                let c = tree.get(*c_id);
                assert!(c.is_dir());
                assert_eq!(Path::new(&c.path), Path::new("./b/c"));
                if let TypeSpecficData::Directory(c_children) = &c.data {
                    assert_eq!(c_children.len(), 1);
                    let d_id = c_children.get("d").expect("d exists");
                    let d = tree.get(*d_id);
                    assert!(d.is_file());
                    assert_eq!(Path::new(&d.path), Path::new("b/c/d"));
                }
            }
        }
    }

    #[test]
    fn tree_construction_with_root_dir() {
        let tree = FileTree::new(
            "b",
            vec![
                ("b/e".to_string(), FileType::File),
                ("b/c/d".to_string(), FileType::File),
            ],
        )
        .unwrap();

        let root = tree.get(tree.root_id);
        assert!(root.is_dir());
        assert_eq!(Path::new(&root.path), Path::new("b"));
        if let TypeSpecficData::Directory(b_children) = &root.data {
            assert_eq!(b_children.len(), 2);
            let c_id = b_children.get("c").expect("c exists");
            let c = tree.get(*c_id);
            assert!(c.is_dir());
            let e_id = b_children.get("e").expect("e exists");
            let e = tree.get(*e_id);
            assert!(e.is_file());
            assert_eq!(Path::new(&c.path), Path::new("b/c"));
            if let TypeSpecficData::Directory(c_children) = &c.data {
                assert_eq!(c_children.len(), 1);
                let d_id = c_children.get("d").expect("d exists");
                let d = tree.get(*d_id);
                assert!(d.is_file());
                assert_eq!(Path::new(&d.path), Path::new("b/c/d"));
            }
        }
    }

    #[test]
    fn metadata_computation() {
        // Tree structure:
        // .
        // ├── a (file)
        // └── b/
        //     └── c/
        //         └── d (file)
        let mut tree = FileTree::new(
            ".",
            vec![
                ("a".to_string(), FileType::File),
                ("b/c/d".to_string(), FileType::File),
            ],
        )
        .unwrap();

        tree.compute_metadata();

        let root = tree.get(tree.root_id);
        // Root has 4 descendants: a, b, c, d
        assert_eq!(root.total_descendants, 4);
        // Root has 1 dir child (b) and 1 file child (a)
        assert_eq!(root.child_dir_count, 1);
        assert_eq!(root.child_file_count, 1);
        assert!(!root.is_skinny()); // has both a file and a dir

        if let TypeSpecficData::Directory(children) = &root.data {
            // Check 'b' directory
            let b_id = children.get("b").unwrap();
            let b = tree.get(*b_id);
            assert_eq!(b.total_descendants, 2); // c, d
            assert_eq!(b.child_dir_count, 1);   // c
            assert_eq!(b.child_file_count, 0);
            assert!(b.is_skinny()); // only one dir child, no files

            // Check 'c' directory
            if let TypeSpecficData::Directory(b_children) = &b.data {
                let c_id = b_children.get("c").unwrap();
                let c = tree.get(*c_id);
                assert_eq!(c.total_descendants, 1); // d
                assert_eq!(c.child_dir_count, 0);
                assert_eq!(c.child_file_count, 1);  // d
                assert!(!c.is_skinny()); // has a file, not a single dir

                // Check 'd' file
                if let TypeSpecficData::Directory(c_children) = &c.data {
                    let d_id = c_children.get("d").unwrap();
                    let d = tree.get(*d_id);
                    assert_eq!(d.total_descendants, 0);
                    assert_eq!(d.child_dir_count, 0);
                    assert_eq!(d.child_file_count, 0);
                }
            }

            // Check 'a' file
            let a_id = children.get("a").unwrap();
            let a = tree.get(*a_id);
            assert_eq!(a.total_descendants, 0);
            assert_eq!(a.child_dir_count, 0);
            assert_eq!(a.child_file_count, 0);
        }
    }

    #[test]
    fn metadata_skinny_path() {
        // Tree structure - a skinny path:
        // .
        // └── src/
        //     └── main/
        //         └── java/
        //             └── com/
        //                 └── App.java (file)
        let mut tree = FileTree::new(
            ".",
            vec![("src/main/java/com/App.java".to_string(), FileType::File)],
        )
        .unwrap();

        tree.compute_metadata();

        let root = tree.get(tree.root_id);
        assert_eq!(root.total_descendants, 5); // src, main, java, com, App.java
        assert!(root.is_skinny()); // only src dir, no files

        // Navigate down the skinny path
        if let TypeSpecficData::Directory(children) = &root.data {
            let src = tree.get(*children.get("src").unwrap());
            assert!(src.is_skinny());
            assert_eq!(src.total_descendants, 4);

            if let TypeSpecficData::Directory(src_children) = &src.data {
                let main = tree.get(*src_children.get("main").unwrap());
                assert!(main.is_skinny());
                assert_eq!(main.total_descendants, 3);

                if let TypeSpecficData::Directory(main_children) = &main.data {
                    let java = tree.get(*main_children.get("java").unwrap());
                    assert!(java.is_skinny());
                    assert_eq!(java.total_descendants, 2);

                    if let TypeSpecficData::Directory(java_children) = &java.data {
                        let com = tree.get(*java_children.get("com").unwrap());
                        assert!(!com.is_skinny()); // has a file, not a dir
                        assert_eq!(com.total_descendants, 1);
                        assert_eq!(com.child_file_count, 1);
                    }
                }
            }
        }
    }
}
