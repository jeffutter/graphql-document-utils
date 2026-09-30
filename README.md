# GraphQL Document Utilities

`graphql-document-utils` rewrites GraphQL documents from the command line:
normalize, focus, and strip queries; format, sort, focus, and prune schemas.

For agents: `graphql-document-utils skill` prints a complete usage guide.

## Common tasks

```bash
# Canonicalize a query for diffing
graphql-document-utils query normalize -q query.graphql
# Keep only the parts of a query that reach a type or field
graphql-document-utils query focus -s schema.graphql -q query.graphql User.email
# Remove a type or field, and everything that references it, from a query
graphql-document-utils query strip -s schema.graphql -q query.graphql Profile
# Cut a schema down to what a query uses
graphql-document-utils schema prune -s schema.graphql -q query.graphql
# Extract a type and everything it depends on from a schema
graphql-document-utils schema focus -s schema.graphql User
# Sort and format a schema
graphql-document-utils schema sort -s schema.graphql
```

Run `graphql-document-utils <noun> <verb> --help` for every rule a command
follows.

## Installation

Download a binary for Linux (x86_64) or macOS (Apple silicon) from the
[GitHub releases page](https://github.com/jeffutter/graphql-document-utils/releases),
or build it with Cargo (Rust 1.85 or later):

```bash
cargo install --locked --git https://github.com/jeffutter/graphql-document-utils
```

To have an agent such as Claude Code load the usage guide whenever a task calls
for the tool, save it as a skill, and save it again after upgrading:

```bash
mkdir -p ~/.claude/skills/graphql-document-utils
graphql-document-utils skill > ~/.claude/skills/graphql-document-utils/SKILL.md
```

## Usage

Every command but `skill` is `graphql-document-utils <noun> <verb>`, where the
noun is `query` or `schema`.

- Every `query` and `schema` command is a filter: it prints a GraphQL
  document to stdout and changes no file, so redirect stdout to save it.
  Errors, warnings, and notes go to stderr, as `error:`, `warning:`, and
  `note:` lines.
- The document a command is named for reads stdin when its flag is omitted
  (`-q` on `query` commands, `-s` on `schema` commands), so commands chain with
  pipes. Outside a pipeline, pass the file: a command fails at once when stdin
  is a terminal, but waits on an open stdin that is never closed.
- Stdin holds one document, so the other is a file. `query focus` and
  `query strip` need `-s SCHEMA`, as a query alone does not say what type each
  field returns. `schema prune` needs `-q QUERY`, or `-q -` to read the query
  from stdin when `-s` names a file.
- Positional arguments are type or field names, never files.
- `#` comments are dropped, and `"descriptions"` kept. A name defined more
  than once warns, and only its first definition is used, except by
  `query normalize`, `schema format`, and `schema sort`, which keep them all.

The examples below run against this schema, `schema.graphql`:

```graphql
type Query {
  user(id: ID!): User
  company: Company
  search(filter: SearchFilter): [SearchResult]
}

"Anything with a global ID."
interface Node {
  id: ID!
}

type User implements Node {
  id: ID!
  name: String
  email: String
  profile: Profile
}

type Profile {
  bio: String
  avatar: Avatar
}

type Avatar {
  url: String
}

type Company implements Node {
  id: ID!
  name: String
  employees(first: Int): [User]
}

# What `search` can find.
union SearchResult = User | Company

input SearchFilter {
  term: String
}
```

and this query, `query.graphql`:

```graphql
query GetUser($id: ID!, $first: Int, $filter: SearchFilter) {
  user(id: $id) {
    name
    profile {
      bio
      avatar {
        url
      }
    }
  }
  company {
    employees(first: $first) {
      name
    }
  }
  search(filter: $filter) {
    __typename
    ... on User {
      email
    }
    ... on Company {
      name
    }
  }
}
```

### `query normalize`

Sorts a query into a canonical form, so queries that differ only in layout, or
in the order of parts with different names, come out the same and can be
diffed or deduplicated. `-m` prints it on one line. No schema is needed.

```bash
graphql-document-utils query normalize -q query.graphql
```

```graphql
query GetUser($filter: SearchFilter, $first: Int, $id: ID!) {
  company {
    employees(first: $first) {
      name
    }
  }
  search(filter: $filter) {
    __typename
    ... on Company {
      name
    }
    ... on User {
      email
    }
  }
  user(id: $id) {
    name
    profile {
      avatar {
        url
      }
      bio
    }
  }
}
```

### `query focus`

Keeps only the paths from an operation root to the selections matching the
targets, with the whole selection at each match, so the output is still a valid
query. An interface or union target matches its implementors or members too.
Fragments, variables, and operations that no longer reach a target are dropped.

```bash
graphql-document-utils query focus -s schema.graphql -q query.graphql Profile
```

```graphql
query GetUser($id: ID!) {
  user(id: $id) {
    profile {
      bio
      avatar {
        url
      }
    }
  }
}
```

### `query strip`

The complement of `query focus`: removes the targets and everything that
references them. A selection set left empty takes its parent with it, and
arguments typed with a stripped input type go too, along with their variables.
A required argument is never removed on its own; the field that needs it goes
instead.

```bash
graphql-document-utils query strip -s schema.graphql -q query.graphql SearchFilter
```

```graphql
query GetUser($id: ID!, $first: Int) {
  user(id: $id) {
    name
    profile {
      bio
      avatar {
        url
      }
    }
  }
  company {
    employees(first: $first) {
      name
    }
  }
  search {
    __typename
    ... on User {
      email
    }
    ... on Company {
      name
    }
  }
}
```

### `schema prune`

Removes the types and fields the query does not use, keeping what the rest
needs, so the query stays valid against the output.

```bash
graphql-document-utils schema prune -s schema.graphql -q query.graphql
```

```graphql
type Query {
  user(id: ID!): User
  company: Company
  search(filter: SearchFilter): [SearchResult]
}

type User {
  name: String
  email: String
  profile: Profile
}

type Profile {
  bio: String
  avatar: Avatar
}

type Avatar {
  url: String
}

type Company {
  name: String
  employees(first: Int): [User]
}

union SearchResult = User | Company

input SearchFilter {
  term: String
}
```

To prune for several queries at once, pass them as one document, such as by
piping them all to `-q -`:

```bash
cat queries/*.graphql | graphql-document-utils schema prune -s schema.graphql -q -
```

### `schema focus`

Keeps the given types and everything reachable from them, plus what that needs
to stand alone (argument and input types, interfaces, directive definitions).
The output defines every type it names, bar one the schema only extends, but it
is a part of the schema rather than one to serve: it has a query root only if a
given type reaches it. It takes type names only; for fields, use `query focus`.

```bash
graphql-document-utils schema focus -s schema.graphql Profile
```

```graphql
type Profile {
  bio: String
  avatar: Avatar
}

type Avatar {
  url: String
}
```

### `schema format` and `schema sort`

`schema format` reprints a schema in a standard layout, in source order.
`schema sort` also orders the definitions: the schema definition, directives,
types, then extensions, each A-Z by name. Fields keep their order. Sort output
is already formatted, so there is no need to run both.

```bash
graphql-document-utils schema format -s schema.graphql
graphql-document-utils schema sort -s schema.graphql > sorted.graphql
```

### Pipelines

Stdin holds one document, so in a pipeline the other document is a file:

```bash
cat query.graphql \
  | graphql-document-utils query strip -s schema.graphql SearchFilter \
  | graphql-document-utils query focus -s schema.graphql Profile \
  | graphql-document-utils query normalize -m

graphql-document-utils schema prune -s schema.graphql -q query.graphql \
  | graphql-document-utils schema focus User \
  | graphql-document-utils schema sort > user.graphql
```

Prune before focusing: `schema focus User` drops `Query`, which leaves
`schema prune` nothing to match the query against.

### Results and exit codes

- `0`: success. A valid target that matches nothing is not an error, but gets
  a `note:` on stderr. When no target matches, `query focus` prints nothing,
  and `query strip` prints the query unchanged apart from dropping unused
  fragments.
- Empty output is zero bytes. A blank document to transform (only whitespace,
  commas, and comments) passes through as empty, so a pipeline survives a
  stage that leaves nothing.
- `1`: bad input, or output that cannot be written: an unreadable file, a parse
  error (with line and column), a blank schema for `query focus` or
  `query strip`, a malformed target (or a file passed as one), an unknown type
  or field (with a did-you-mean when a name is close), a field or built-in
  scalar given to `schema focus`, or a failed write to stdout (as on a full
  disk).
- `2`: usage error: an unknown command or flag, a missing flag or targets, no
  document to read (no flag, and stdin a terminal), or both of `schema prune`'s
  documents on stdin.

## Library

Query normalization is also a Rust library,
[`graphql-normalize`](graphql-normalize-lib/README.md), on
[crates.io](https://crates.io/crates/graphql-normalize).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
