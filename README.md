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

1. Pick the command for the task from the per-command sections below. For
   every rule of one command, run `graphql-document-utils <noun> <verb> --help`.
2. Pass the documents.
   - Pass the query to a `query` command with `-q FILE`.
   - Pass the schema to a `schema` command with `-s FILE`.
   - Omit the `-q` of a `query` command, or the `-s` of a `schema` command,
     to read that document from stdin, as in a pipeline.
   - Outside a pipeline, pass every document with its flag. A command waits
     forever on a stdin pipe that never closes.
   - Pass `query focus` and `query strip` the schema too, with `-s FILE`.
   - Pass `schema prune` the query too, with `-q FILE`. To pipe the query in,
     pass `-q -` and the schema with `-s FILE`.
3. Pass targets to `query focus`, `query strip`, and `schema focus` as
   positional arguments.
   - Pass `query focus` and `query strip` types (`Profile`) or fields on a
     type (`User.email`).
   - Pass `schema focus` types. To target a field, use `query focus`.
   - Spell each name as the schema does, case included. Names from `extend`
     blocks count.
   - Pass files only with `-q` and `-s`. A positional argument is a name.
   - An unknown name exits 1, with a did-you-mean when a name is close.
   - An interface or union target also matches its implementors or members.
   - A field target matches across interfaces both ways. When `User`
     implements `Person`, `Person.name` matches `name` on `User`, and
     `User.name` matches `name` on `Person`.
   - Fields the schema does not define, such as `__typename`, match no target.
   - `query focus` matches output selections only. An input type target
     matches nothing.
   - `query strip` also removes arguments and input fields of a stripped type,
     and their variables.
   - `query strip` removes a field or inline fragment whose selection set it
     empties, and an operation it empties.
   - `query strip` removes a field or directive whose required input it
     removes.
4. Redirect stdout to a file to save the result.
5. Read the result.
   - Stdout holds only the GraphQL document.
   - Stderr holds `error:`, `warning:`, and `note:` lines.
   - Look up the exit code under Results and exit codes below.
   - On exit 1 or 2, fix what the `error:` line names, then rerun.
   - The output drops `#` comments and keeps `"descriptions"`.
   - A name defined more than once gets a `warning:`, and the command uses
     its first definition. `query normalize`, `schema format`, and
     `schema sort` keep every definition.

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

- `0`: success. A valid target that matches nothing adds a `note:` on stderr.
  When no target matches, `query focus` prints nothing, and `query strip`
  prints the query minus its unused fragments.
- Empty output is zero bytes. A blank document to transform passes through as
  empty, so the next command in a pipeline still runs. Blank means only
  whitespace, commas, and comments.
- `1`: bad input, or output that cannot be written: an unreadable file, a parse
  error (with line and column), a blank schema for `query focus` or
  `query strip`, a malformed target (or a file passed as one), an unknown type
  or field, a field or built-in scalar given to `schema focus`, or a failed
  write to stdout (as on a full disk).
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
