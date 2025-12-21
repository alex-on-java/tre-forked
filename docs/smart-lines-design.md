# Smart Line-Limited Output (`--lines`)

## Problem Statement

The existing `--limit` (`-l`) flag controls output by **depth**, but depth is a poor proxy for output size:

```bash
# Unpredictable output size:
tre --limit 3    # Could be 20 lines or 2000 lines
tre --limit 4    # Might explode if there's a dense directory at level 3
```

A directory like `src/main/java/de/konux/healthservice/` is 6 levels deep but contains only one path — depth-limiting cuts it off uselessly. Meanwhile, a flat `node_modules/` at depth 2 can produce thousands of lines.

**AI coding agents need bounded, predictable output** that fits context windows while maximizing information value.

## Goal

Provide a `--lines N` flag that produces approximately N lines of output while **maximizing the usefulness** of those lines for understanding the codebase structure.

## Core Principles

### 1. Predictable Output Size
The output should be close to the requested line count. Users (and AI agents) should be able to reason about context window usage.

### 2. Structure Over Bulk
Prioritize showing the **shape** of the repository over showing individual files:
- Top-level directories should always be visible
- Deep but sparse branches are more valuable than dense flat directories
- A single file buried 10 levels deep is interesting; 500 files in one directory is noise

### 3. No Information Black Holes
Never completely hide a significant part of the tree. If a directory is collapsed, indicate its size so readers know something exists there.

### 4. Skinny Path Awareness
Directory chains with single children (like Java package paths) should be treated specially — they contain little information per level and can be collapsed or displayed compactly.

## Key Concepts

### Skinny Paths
A "skinny" directory has exactly one child directory and zero files:

```
src/                    # skinny (only main/)
└── main/               # skinny (only java/)
    └── java/           # skinny (only de/)
        └── de/         # skinny (only konux/)
            └── konux/  # skinny (only healthservice/)
                └── healthservice/
                    ├── core/           # NOT skinny (has siblings)
                    ├── infrastructure/
                    └── presentation/
```

These paths are structurally important but informationally sparse. Expanding them shouldn't "cost" much against the line budget.

### Information Density
Not all lines are equal:
- `src/main/java/de/` — low information (just namespace boilerplate)
- `infrastructure/` — high information (architectural boundary)
- One of 500 files in a directory — low marginal information

### Dense vs Sparse Directories
- **Sparse**: Few children, worth expanding fully
- **Dense**: Many children, worth summarizing with counts

## Requirements

### Functional Requirements

1. **Flag**: `-n` / `--lines` accepting a positive integer
2. **Approximate bound**: Output should be close to N lines (exact match not required)
3. **Graceful degradation**: Small repos (< N lines) should display normally
4. **Truncation indication**: When content is omitted, show what's missing (counts, ellipsis, etc.)

### Non-Functional Requirements

1. **Performance**: Building the full tree in memory is acceptable; optimize for output quality, not speed
2. **Determinism**: Same input should produce same output (no random sampling)
3. **Composability**: Should work with other flags (`-a`, `-d`, `-E`, etc.)

## Examples

### Example 1: Small Repository (No Truncation Needed)

```bash
$ tre --lines 100
```

Repository has 30 entries — shows everything normally, no truncation.

### Example 2: Medium Repository with Dense Directory

**Without --lines:**
```
project/
├── src/
│   ├── components/
│   │   ├── Button.tsx
│   │   ├── Card.tsx
│   │   ├── ... (200 more files)
│   │   └── Tooltip.tsx
│   └── utils/
│       └── helpers.ts
├── node_modules/     # 50,000 files if -a flag used
└── package.json
```

**With --lines 50 (desired behavior):**
```
project/
├── src/
│   ├── components/     ... (203 files)
│   └── utils/
│       └── helpers.ts
├── node_modules/       ... (50000 entries)
└── package.json
```

The output prioritizes showing that `components/` and `node_modules/` exist and their scale, rather than listing files.

### Example 3: Java Project with Skinny Paths

**Without --lines:**
```
healthservice/
└── src/
    └── main/
        └── java/
            └── de/
                └── konux/
                    └── healthservice/
                        ├── core/
                        │   ├── application/
                        │   │   └── ... (many files)
                        │   ├── domain/
                        │   └── ports/
                        ├── infrastructure/
                        │   └── ... (many packages)
                        └── presentation/
                            └── rest/
```

**With --lines 30 (desired behavior):**
```
healthservice/
└── src/main/java/de/konux/healthservice/    # Skinny path collapsed
    ├── core/
    │   ├── application/    ... (12 files)
    │   ├── domain/         ... (8 files)
    │   └── ports/          ... (5 files)
    ├── infrastructure/     ... (45 files, 8 packages)
    └── presentation/
        └── rest/           ... (15 files)
```

The skinny path `src/main/java/de/konux/healthservice/` is shown on one line. The architectural packages (`core`, `infrastructure`, `presentation`) are always visible.

### Example 4: Deep Sparse Branch Should Expand

```
infra/
├── k8s/
│   ├── helm/
│   │   ├── Chart.yaml
│   │   ├── common-values.yaml
│   │   └── templates/
│   │       ├── deployment.yaml
│   │       ├── service.yaml
│   │       └── hpa.yaml
│   └── values/
│       ├── prd-values.yaml
│       ├── stg-values.yaml
│       └── test-values.yaml
└── openapi/
    └── openapi.yaml
```

This entire tree (22 lines) should expand fully even with `--lines 50` because it's sparse and every file is meaningful. The algorithm shouldn't truncate sparse branches just to save lines.

### Example 5: Top-Level Always Visible

Even with a very small budget, top-level directories should be visible:

```bash
$ tre --lines 10
```

```
large-monorepo/
├── packages/           ... (500 entries)
├── apps/               ... (200 entries)
├── libs/               ... (150 entries)
├── tools/              ... (50 entries)
├── docs/               ... (30 entries)
├── package.json
├── tsconfig.json
└── README.md
```

The user sees the high-level structure even though details are hidden.

## Metadata Available

Each tree node has these computed fields available for algorithm use:

| Field | Description |
|-------|-------------|
| `total_descendants` | Recursive count of all entries below this node |
| `child_dir_count` | Number of direct child directories |
| `child_file_count` | Number of direct child files |
| `is_skinny()` | `true` if `child_dir_count == 1 && child_file_count == 0` |

## Open Questions for Implementation

1. **How to display collapsed directories?** Options: `... (N)`, `... (N files)`, `... (N files, M dirs)`, inline vs separate line
2. **How to display collapsed skinny paths?** Options: `src/main/java/.../healthservice/`, `src/main/java/de/konux/healthservice/`, or something else
3. **Ordering within directories**: Alphabetical? Directories first? Largest first?
4. **Interaction with `--limit`**: Should both flags work together? Which takes precedence?
5. **Edge cases**: Empty directories, symlinks, root with 1000 direct children

## Success Criteria

A good implementation should:
- [ ] Show bounded output close to requested line count
- [ ] Always reveal top-level structure
- [ ] Expand sparse/deep branches fully when budget allows
- [ ] Collapse dense directories with size indication
- [ ] Handle skinny paths elegantly
- [ ] Produce deterministic output
- [ ] Feel intuitive — output should match what a human would manually select
