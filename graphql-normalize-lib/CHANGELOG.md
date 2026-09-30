# Changelog

All notable changes to `graphql-normalize` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Changed

- `normalize` no longer reorders list values. List items keep their source
  order, because that order is meaningful (`orderBy: [...]`, positional
  inputs). Before, every list in a variable default or directive argument was
  sorted by a string key, so `[10, 2, 1]` came out as `[1, 10, 2]`. Output for
  any query containing such a list changes.
- Field argument values, directive argument values, and variable defaults are
  now treated the same way. Input object fields are sorted by key in all three
  positions, including objects nested inside lists.
