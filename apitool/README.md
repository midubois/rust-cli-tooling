# apitool

A curl-like HTTP client for the terminal with automatic request history and saved aliases.

## Install

```bash
cargo install --path .
```

## Usage

```
apitool <COMMAND> [OPTIONS]
```

---

## Making Requests

```
apitool get|post|put|patch|delete <URL> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-H KEY:VALUE` | Add a request header (repeatable) |
| `-q KEY=VALUE` | Add a query parameter (repeatable) |
| `-d '{"k":"v"}'` | JSON request body |
| `-f body.json` | Read JSON body from a file |
| `-i` | Print response headers |
| `--save-as NAME` | Save this request as a named alias |

Every request is automatically saved to `~/.apitool/history.json`.

### Examples

```bash
# Simple GET
apitool get https://api.example.com/users

# GET with query params and auth header
apitool get https://api.example.com/users \
  -q page=1 -q limit=10 \
  -H "Authorization:Bearer my-token"

# POST with inline JSON body
apitool post https://api.example.com/users \
  -H "Authorization:Bearer my-token" \
  -d '{"name":"Alice","email":"alice@example.com"}'

# POST body from file
apitool post https://api.example.com/users -f ./payload.json

# Save a request as an alias for later
apitool get https://api.example.com/me \
  -H "Authorization:Bearer my-token" \
  --save-as get-me
```

---

## Aliases

Aliases are named shortcuts saved to `~/.apitool/aliases.json`.

```bash
# Run a saved alias
apitool run get-me

# Override query params when running an alias
apitool run get-users -q page=2

# Override a header when running an alias
apitool run get-users -H "Authorization:Bearer other-token"

# List all saved aliases
apitool alias list

# Inspect an alias
apitool alias show get-me

# Delete an alias
apitool alias delete get-me
```

---

## History

```bash
# Show last 20 requests (default)
apitool history

# Show last 5 requests
apitool history -n 5

# Clear all history
apitool history --clear
```

History output columns: `#  time (UTC)  method  status  ms  url  [alias]`

---

## Storage

| Path | Contents |
|------|----------|
| `~/.apitool/history.json` | All requests and responses (capped at 1 000) |
| `~/.apitool/aliases.json` | Named request shortcuts |

---

## Response Display

- **Spinner** shows while the request is in flight.
- **Status line** is color-coded: green (2xx), yellow (3xx), red (4xx/5xx).
- **JSON bodies** are pretty-printed with syntax highlighting.
- **Non-JSON bodies** are printed as-is.
