use super::file_tree::{File, FileTree, FileType};
use std::cmp::Ordering;
use std::fs;

#[derive(Debug, Clone, PartialEq)]
pub struct FormattedEntry {
    pub name: String,
    pub path: String,
    pub prefix: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone)]
struct PlannedNode {
    name: String,
    path: String,
    link: Option<String>,
    children: Vec<PlannedNode>,
}

#[derive(Debug)]
struct PlannedChildren {
    nodes: Vec<PlannedNode>,
    used_lines: usize,
}

struct Planner<'a> {
    tree: &'a FileTree,
    make_absolute: bool,
    max_lines: Option<usize>,
}

fn build_full_plan(tree: &FileTree, make_absolute: bool) -> PlannedNode {
    fn helper(tree: &FileTree, id: usize, make_absolute: bool) -> PlannedNode {
        let file = tree.get(id);
        let mut children = Vec::new();
        if let Some(child_map) = file.children() {
            for child_id in child_map.values() {
                children.push(helper(tree, *child_id, make_absolute));
            }
        }

        PlannedNode {
            name: file.display_name.clone(),
            path: canonical_path(file, make_absolute),
            link: file.link(),
            children,
        }
    }

    helper(tree, tree.root_id, make_absolute)
}

fn canonical_path(file: &File, make_absolute: bool) -> String {
    if make_absolute {
        fs::canonicalize(&file.path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| file.path.clone())
    } else {
        file.path.clone()
    }
}

impl<'a> Planner<'a> {
    fn new(tree: &'a FileTree, make_absolute: bool, max_lines: Option<usize>) -> Self {
        Self {
            tree,
            make_absolute,
            max_lines,
        }
    }

    fn build_plan(&self) -> PlannedNode {
        let budget = self.max_lines.unwrap_or(usize::MAX);
        let (mut node, used) = self.plan_node(self.tree.get_root(), budget, 0);

        // If we were extremely constrained and could not even fit root children, annotate root
        // to avoid a total information black hole.
        if used == 1 {
            let total_missing = self.tree.get_root().total_descendants;
            if total_missing > 0 {
                node.name = format!("{}    ... ({} entries)", node.name, total_missing);
            }
        }

        node
    }

    fn score(&self, file: &File, depth: usize) -> f64 {
        let branching = (file.child_dir_count + file.child_file_count + 1) as f64;
        let sparsity = 1.0 / branching;
        let scale = (file.total_descendants as f64 + 2.0).ln_1p();
        let skinny_bonus = if file.is_skinny() { 1.6 } else { 1.0 };
        let dir_bonus = match file.file_type {
            FileType::Directory => 2.2,
            FileType::File => 0.8,
            FileType::Link => 0.6,
        };
        let depth_bonus = 1.0 + (depth as f64 * 0.12);

        (sparsity + scale) * skinny_bonus * dir_bonus * depth_bonus
    }

    fn collapse_skinny_path<'b>(&'b self, start: &'b File) -> (&'b File, String) {
        let mut labels = vec![start.display_name.clone()];
        let mut cursor = start;

        while cursor.is_skinny() {
            if let Some(children) = cursor.children() {
                if let Some((_, next_id)) = children.iter().next() {
                    let next = self.tree.get(*next_id);
                    if let FileType::Directory = next.file_type {
                        labels.push(next.display_name.clone());
                        cursor = next;
                        continue;
                    }
                }
            }
            break;
        }

        let raw_label = labels.join("/");

        let label = raw_label
            .strip_prefix("./")
            .map(|s| s.to_string())
            .unwrap_or(raw_label);

        (cursor, label)
    }

    fn decorated_name(&self, _file: &File, collapsed_label: String) -> String {
        collapsed_label
    }

    fn plan_node(&self, file: &File, budget: usize, depth: usize) -> (PlannedNode, usize) {
        let (logical, collapsed_label) = self.collapse_skinny_path(file);
        let mut node = PlannedNode {
            name: self.decorated_name(logical, collapsed_label),
            path: canonical_path(logical, self.make_absolute),
            link: logical.link(),
            children: Vec::new(),
        };

        if !matches!(logical.file_type, FileType::Directory) || budget <= 1 {
            if matches!(logical.file_type, FileType::Directory)
                && budget <= 1
                && logical.total_descendants > 0
            {
                node.name = format!("{}    ... ({} entries)", node.name, logical.total_descendants);
            }
            return (node, 1);
        }

        let children = logical
            .children()
            .map(|c| c.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if children.is_empty() {
            return (node, 1);
        }

        let forced_full_children = depth == 0; // root children should always be visible
        let planned_children =
            self.plan_children(logical, children, budget - 1, depth + 1, forced_full_children);
        node.children = planned_children.nodes;
        (node, 1 + planned_children.used_lines)
    }

    fn plan_children(
        &self,
        parent: &File,
        children: Vec<usize>,
        budget: usize,
        depth: usize,
        force_show_all: bool,
    ) -> PlannedChildren {
        if children.is_empty() {
            return PlannedChildren {
                nodes: Vec::new(),
                used_lines: 0,
            };
        }

        let mut scored_children: Vec<(usize, f64)> = children
            .iter()
            .map(|id| (*id, self.score(self.tree.get(*id), depth)))
            .collect();

        scored_children.sort_by(|(a_id, a_score), (b_id, b_score)| {
            b_score
                .partial_cmp(a_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| {
                    let a_name = &self.tree.get(*a_id).display_name;
                    let b_name = &self.tree.get(*b_id).display_name;
                    a_name.cmp(b_name)
                })
        });

        let total_children = scored_children.len();
        let visible_children = if force_show_all {
            total_children
        } else {
            total_children.min(budget.max(1))
        };

        let (mut to_display, hidden): (Vec<_>, Vec<_>) = scored_children
            .into_iter()
            .enumerate()
            .partition(|(idx, _)| *idx < visible_children);

        to_display.sort_by_key(|(idx, _)| *idx); // restore deterministic slicing order
        let display_only = to_display
            .into_iter()
            .map(|(_, data)| data)
            .collect::<Vec<_>>();
        let hidden_ids = hidden
            .into_iter()
            .map(|(_, (id, _))| id)
            .collect::<Vec<_>>();

        let mut result_nodes = Vec::new();
        let mut used_lines = 0;

        let effective_budget = budget.max(display_only.len());
        let expandable_pool = effective_budget.saturating_sub(display_only.len());
        let total_score: f64 = display_only.iter().map(|(_, score)| score).sum();

        for (_idx, (child_id, child_score)) in display_only.into_iter().enumerate() {
            let mut allowance = 1usize;
            if expandable_pool > 0 && total_score > 0.0 {
                let share = ((child_score / total_score) * expandable_pool as f64).ceil() as usize;
                allowance += share;
            }

            let (child_plan, used) = self.plan_node(self.tree.get(child_id), allowance, depth);
            used_lines += used;
            result_nodes.push(child_plan);

            // If we've already used more than budget but must keep all top-level nodes, keep going.
            if used_lines >= effective_budget && !force_show_all {
                break;
            }
        }

        if !hidden_ids.is_empty() {
            let mut hidden_entries = 0usize;
            let mut hidden_dirs = 0usize;
            let mut hidden_files = 0usize;
            for child_id in hidden_ids.iter() {
                let child = self.tree.get(*child_id);
                hidden_entries += 1 + child.total_descendants;
                match child.file_type {
                    FileType::Directory => hidden_dirs += 1,
                    _ => hidden_files += 1,
                }
            }

            let summary_name = format!(
                "{} ... {} more ({} dirs, {} files)",
                parent.display_name, hidden_entries, hidden_dirs, hidden_files
            );
            result_nodes.push(PlannedNode {
                name: summary_name,
                path: canonical_path(parent, self.make_absolute),
                link: None,
                children: Vec::new(),
            });
            used_lines += 1;
        }

        PlannedChildren {
            nodes: result_nodes,
            used_lines,
        }
    }
}

fn emit_entries(node: &PlannedNode, lineage: &mut Vec<bool>, result: &mut Vec<FormattedEntry>) {
    let prefix = build_prefix(lineage);
    result.push(FormattedEntry {
        name: node.name.clone(),
        path: node.path.clone(),
        prefix,
        link: node.link.clone(),
    });

    for (idx, child) in node.children.iter().enumerate() {
        let has_sibling = idx + 1 < node.children.len();
        lineage.push(has_sibling);
        emit_entries(child, lineage, result);
        lineage.pop();
    }
}

fn build_prefix(lineage: &[bool]) -> String {
    if lineage.is_empty() {
        return String::new();
    }

    let mut prefix = String::new();
    for (idx, has_next) in lineage.iter().enumerate() {
        if idx + 1 == lineage.len() {
            prefix.push_str(if *has_next { "├── " } else { "└── " });
        } else {
            prefix.push_str(if *has_next { "│   " } else { "    " });
        }
    }
    prefix
}

pub fn format_paths(
    root_path: &str,
    children: Vec<(String, FileType)>,
    make_absolute: bool,
    max_lines: Option<usize>,
) -> Vec<FormattedEntry> {
    let mut result = Vec::new();
    match FileTree::new(root_path, children) {
        Some(mut tree) => {
            let plan_root = if max_lines.is_some() {
                tree.compute_metadata();
                let planner = Planner::new(&tree, make_absolute, max_lines);
                planner.build_plan()
            } else {
                build_full_plan(&tree, make_absolute)
            };
            emit_entries(&plan_root, &mut Vec::new(), &mut result);
            result
        }
        None => result,
    }
}

#[cfg(test)]
mod test {
    use super::FormattedEntry;
    use crate::file_tree::FileType;
    use std::path;

    #[test]
    fn formatting_works() {
        let formatted = super::format_paths(
            ".",
            vec![
                ("a".to_string(), FileType::File),
                (format!("b{}c", path::MAIN_SEPARATOR), FileType::File),
            ],
            false,
            None,
        );

        let bc_path = format!("b{}c", path::MAIN_SEPARATOR);
        let variant_b_first = vec![
            FormattedEntry {
                name: ".".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: format!(".{}b", path::MAIN_SEPARATOR),
                prefix: "├── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "│   └── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "└── ".to_string(),
                link: None,
            },
        ];

        let variant_a_first = vec![
            FormattedEntry {
                name: ".".to_string(),
                path: ".".to_string(),
                prefix: String::new(),
                link: None,
            },
            FormattedEntry {
                name: "a".to_string(),
                path: "a".to_string(),
                prefix: "├── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "b".to_string(),
                path: format!(".{}b", path::MAIN_SEPARATOR),
                prefix: "└── ".to_string(),
                link: None,
            },
            FormattedEntry {
                name: "c".to_string(),
                path: bc_path.clone(),
                prefix: "    └── ".to_string(),
                link: None,
            },
        ];

        assert!(formatted == variant_a_first || formatted == variant_b_first);
    }

    #[test]
    fn skinny_paths_collapse_under_line_budget() {
        let entries = vec![
            (
                "src/main/java/de/konux/healthservice/core/mod.rs".to_string(),
                FileType::File,
            ),
            (
                "src/main/java/de/konux/healthservice/infrastructure/lib.rs".to_string(),
                FileType::File,
            ),
            (
                "src/main/java/de/konux/healthservice/presentation/rest.rs".to_string(),
                FileType::File,
            ),
        ];

        let formatted = super::format_paths(".", entries, false, Some(6));
        let skinny_line = formatted
            .iter()
            .find(|entry| entry.name.contains("src/main/java/de/konux/healthservice"))
            .cloned()
            .expect("collapsed skinny path should appear");

        assert!(skinny_line
            .name
            .starts_with("src/main/java/de/konux/healthservice"));
        assert!(formatted.len() <= 8);
    }

    #[test]
    fn dense_directories_are_summarized() {
        let mut entries = vec![("README.md".to_string(), FileType::File)];
        for i in 0..20 {
            entries.push((format!("components/Component{}.tsx", i), FileType::File));
        }
        entries.push(("utils/helpers.ts".to_string(), FileType::File));

        let formatted = super::format_paths("project", entries, false, Some(8));
        let summary = formatted
            .iter()
            .find(|entry| entry.name.contains("..."))
            .expect("dense directory summary should exist");

        assert!(summary.name.contains("components"));
        assert!(formatted.len() <= 10);
    }
}
