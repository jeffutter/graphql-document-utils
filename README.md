# GraphQL Document Utilities

This tool provides utilities to process GraphQL queries and schema documents.

## Features

- **Normalization**: Formats and sorts GraphQL queries for better readability.
- **Pruning Unused Types**: Removes types from the schema that are not referenced in any query.
- **Pruning Unused Fields**: Removes fields from types that are not selected in any query.
- **Focus**: Removes any type from a schema that isn't a descendant of a specified type.

## Usage

### Installation

`cargo install graphql-document-utils`

### Commands

#### Query Commands

- **Normalize**

  ```sh
  graphql-document-utils query normalize --path <path> --minify
  ```

#### Schema Commands

- **Format**

  ```sh
  graphql-document-utils schema format --schema <path>
  ```

- **Focus**

  ```sh
  graphql-document-utils schema focus --schema <path> --type <type>
  ```

- **Prune**

  ```sh
  graphql-document-utils schema prune --schema <path> --query <path>
  ```
