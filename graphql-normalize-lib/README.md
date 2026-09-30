# graphql-normalize

Normalize GraphQL queries: parse a query document and print it back in a
canonical order and layout. Queries that differ only in the order of their
fields, arguments, or definitions, or in whitespace, normalize to the same
text, so the result can be diffed, hashed, or used to find duplicates.

It is the library behind `query normalize` in
[graphql-document-utils](https://github.com/jeffutter/graphql-document-utils).

## Install

```bash
cargo add graphql-normalize
```

## Usage

The crate has one function,
`normalize(&str) -> Result<String, Box<dyn std::error::Error>>`. It takes a
query document, which may hold several operations and fragments, and returns
it normalized, ending in a newline. The error is the parse error for a
document that is not a valid query.

```rust
use graphql_normalize::normalize;

let query = r#"
    query User($id: ID!, $avatarSize: Int) {
      user(id: $id) {
        name
        avatar(size: $avatarSize, format: PNG)
        ...UserFriends
      }
    }

    fragment UserFriends on User { friends { name id } }
"#;

let expected = "\
query User($avatarSize: Int, $id: ID!) {
  user(id: $id) {
    avatar(format: PNG, size: $avatarSize)
    name
    ...UserFriends
  }
}

fragment UserFriends on User {
  friends {
    id
    name
  }
}
";

assert_eq!(normalize(query).unwrap(), expected);
assert!(normalize("query {").is_err());
```

## What normalization does

- Operations come first, queries then mutations then subscriptions, each
  sorted by name, followed by fragments sorted by name.
- In a selection set, fields are sorted by name (not alias), followed by
  fragment spreads, then inline fragments by type condition.
- Arguments, directives, and variable definitions are sorted by name, and
  input object fields by key, wherever they appear.
- Names compare without regard to case.
- List values keep their order, since it can be meaningful to the server
  (`orderBy: [...]`, positional inputs).
- The document is printed in `graphql-parser`'s standard layout: two-space
  indentation, one selection per line.
- Nothing is added or removed, except `#` comments, which are dropped.

## Changelog

See
[CHANGELOG.md](https://github.com/jeffutter/graphql-document-utils/blob/main/graphql-normalize-lib/CHANGELOG.md).

## License

MIT
