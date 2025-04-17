# GraphQL Utilities

This tool provides utilities to process GraphQL queries and schema documents.

## Features

- **Normalization**: Formats and sorts GraphQL queries for better readability.
- **Pruning Unused Types**: Removes types from the schema that are not referenced in any query.
- **Pruning Unused Fields**: Removes fields from types that are not selected in any query.
- **Focus**: Removes any type from a schema that isn't a descendant of a specified type.
