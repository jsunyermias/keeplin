# Changelog

All notable changes to keeplin (keeplin-core + keeplin-daemon) are documented
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

Client↔server compatibility is negotiated at runtime via keeplin-srv's
`GET /version` handshake (`protocol_version` + `capabilities`), so the crate
version and the wire protocol version move independently.

## [Unreleased]

### Added
- iCalendar import reads **every** `VEVENT`/`VTODO` in a file, not just the
  first (`from_ics_all`, `import_todos`); the daemon import endpoints accept a
  whole calendar (#107).
- Optional `sync_interval_secs` daemon config: run a relay sync cycle on a
  cadence instead of only when a frontend polls (#111).
- The client negotiates keeplin-srv capabilities via `GET /version` and skips
  features the server does not advertise (keeplin#114).
- Collaborative note discovery pages through `GET /api/notes` with
  `?limit=&cursor=`, following the server's `X-Next-Cursor` header, so a large
  account is not fetched in a single unbounded response. Back-compatible with a
  server that predates pagination (keeplin-srv#29).

### Changed
- Server-backed note/notebook history in server mode: `DbBackend` fetches the
  server's history (every device's changes) with a local-journal fallback, and
  latches a `404`/absent capability to avoid wasted round-trips (#100 follow-up, #113).
- `CollabBackend` checks the HTTP status of the note POST/PATCH mirror and logs
  a server rejection instead of treating it as delivered (#112).
- The alias index no longer holds every Inbox note id; the uuid→Inbox check is a
  backend read at write time, so the index is bounded by the alias count (#106).
- Contact/event by-UID operations resolve from resource **metadata** (the
  `<uid>.vcf`/`.ics` file name) instead of scanning every payload (#105).

### Documentation
- `SECURITY.md` documents that collaborative mode stores note title/body in
  cleartext on the server, and how to avoid it (#110).

## [0.1.0]

- Initial client: `FsBackend` (Syncthing) and `DbBackend` (server relay +
  collaborative channel) storage, at-rest encryption, notebooks/tags/resources,
  links & aliases, ordering/pinning/starring, history & revert, vCard/iCalendar
  interop, and the gRPC + REST/WebSocket daemon surfaces.
