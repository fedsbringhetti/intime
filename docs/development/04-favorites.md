# 4. Favorites

> Phase 1, step 4 · PR [#6](https://github.com/fedsbringhetti/intime/pull/6)

## Goal

Save favorite stops with a label, a line filter and a manual order: `GET` and `POST /api/favorites`, `PATCH` and `DELETE /api/favorites/{id}`. These are the first endpoints that receive a JSON body and change data.

## Decisions

| Decision | Why |
|----------|-----|
| **No accounts** | Single user (cross-cutting decision 4 in `PLAN.md`). |
| **Store only the stop id**, read details from the index at every request | Details stay current after each sync; nothing is duplicated. |
| **No foreign key** to `stops` | Each sync replaces the whole stops table; a foreign key would block it or delete favorites. |
| **Keep favorites whose stop disappeared**, flagged `missing: true` | Silently hiding them would look like data loss. |
| **0-based, contiguous `position`** | The UI can show favorites in order and move one with a single request. |
| **Creation requires an existing stop** | A typo would otherwise create a favorite that never works. |
| **`PATCH` changes only the fields sent; `null` clears the label** | Partial updates without resending everything. |
| **Unknown fields in a body are rejected** | A typo such as `"lable"` gives a 400 instead of being ignored. |

## Files

| File | Role |
|------|------|
| `migrations/0002_favorites.sql` | table `favorites` |
| `src/favorites.rs` | `FavoriteStore`: list, get, create, update, delete, ordering |
| `src/api/favorites.rs` | handlers, request bodies, input cleaning |
| `src/api/error.rs` | `favorite_not_found` |
| `tests/favorites.rs`, `tests/common/mod.rs` | integration tests; shared `synced_app` and `send` helpers |

## How the code works

### Storage

```sql
CREATE TABLE favorites (
    id INTEGER PRIMARY KEY,
    stop_id TEXT NOT NULL,      -- no foreign key, the stop may disappear after a sync
    label TEXT,
    lines TEXT NOT NULL,        -- JSON array; empty means all lines
    position INTEGER NOT NULL,  -- 0-based, kept contiguous
    created_at TEXT NOT NULL
);
```

Every change that touches positions runs in a transaction, so two requests never leave gaps or duplicates.

- **Create** appends at the end: `SELECT COALESCE(MAX(position) + 1, 0)` then `INSERT`.
- **Delete** removes the row and closes the gap:

```rust
let deleted: Option<(i64,)> =
    sqlx::query_as("DELETE FROM favorites WHERE id = ? RETURNING position")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
let Some((position,)) = deleted else {
    return Ok(false);
};
sqlx::query("UPDATE favorites SET position = position - 1 WHERE position > ?")
    .bind(position)
    .execute(&mut *tx)
    .await?;
```

`RETURNING` gives back the position of the deleted row in the same statement. `let ... else` handles the "not found" case and returns early.

- **Move** reads the ids in order, computes the new order with a pure function, and rewrites every position:

```rust
fn move_to(ids: &[i64], id: i64, position: usize) -> Vec<i64> {
    let mut ids: Vec<i64> = ids.iter().copied().filter(|other| *other != id).collect();
    ids.insert(position.min(ids.len()), id);
    ids
}
```

Keeping the logic in a pure function makes it easy to unit test; `min` moves a position past the end to the last place.

### Request bodies

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateFavorite {
    /// Missing: unchanged; `null`: cleared.
    #[serde(default, deserialize_with = "present")]
    label: Option<Option<String>>,
    lines: Option<Vec<String>>,
    position: Option<usize>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
```

`serde` cannot tell a missing field from `null` with a plain `Option`. The double `Option` does:

| JSON | `label` |
|------|---------|
| field absent | `None` (via `default`): leave unchanged |
| `"label": null` | `Some(None)`: clear |
| `"label": "Home"` | `Some(Some("Home"))`: set |

`present` runs only when the field exists in the JSON, and wraps whatever it read in `Some`.

The body is taken as `Result<Json<...>, JsonRejection>` so malformed JSON, wrong types and unknown fields all become our `invalid_request`. The same applies to the `{id}` path parameter (`PathRejection`), so `/api/favorites/abc` gives a JSON 400.

### Cleaning input

- `clean_label`: trims; empty means no label; at most 100 characters.
- `clean_lines`: trims, drops empty entries and duplicates ignoring case (`9B` and `9b` count once), at most 20 lines of at most 10 characters.

### Responses

Each favorite is returned with its stop:

```rust
async fn view(
    state: &AppState,
    favorite: Favorite,
    index_ready: bool,
) -> Result<FavoriteView, ApiError> {
    let stop = state.stops.get(&favorite.stop_id).await?;
    Ok(FavoriteView {
        // Before the first sync every stop would look missing.
        missing: index_ready && stop.is_none(),
        stop,
        favorite,
    })
}
```

`FavoriteView` uses `#[serde(flatten)]` so the favorite's fields sit at the top level next to `stop` and `missing`.

## Tests

- **Unit**: `move_to` (to the front, to the back, past the end), label and line cleaning, missing field vs `null` vs value.
- **Integration** (`tests/favorites.rs`), on top of `synced_app` (the stop index already loaded from the fixture):
  - creation returns 201 with cleaned label and lines and the stop details;
  - new favorites go to the end;
  - moving one shifts the others; `null` clears the label without touching the lines; a position past the end moves it last;
  - deleting closes the gap; deleting twice gives 404;
  - a re-sync without stop 40 keeps its favorite with `missing: true` and `stop: null`;
  - unknown stop and unknown favorite give 404; creation before the first sync gives 503;
  - invalid bodies and paths give 400.

The shared helper `send` builds requests with any method and an optional JSON body.

## Check it by hand

```bash
curl -s -H "Content-Type: application/json" -X POST http://localhost:8080/api/favorites -d '{"stop": "39", "label": "Home", "lines": ["9B", "52"]}'
```

```bash
curl -s -H "Content-Type: application/json" -X PATCH http://localhost:8080/api/favorites/1 -d '{"position": 0, "label": null}'
```

```bash
curl -s http://localhost:8080/api/favorites
```

Restart the service and list again: favorites and their order are kept, and the new migration was applied automatically to the existing database.

## Pitfalls

- **Missing vs `null`** needs the double `Option` described above.
- **Axum rejections** are plain text by default; taking extractors as `Result` keeps every error in the JSON format.
- **Test helpers**: the "synced app" setup moved from `tests/stops.rs` to `tests/common` so favorites tests could reuse it.

## Later changes

- [Step 5](05-robustness.md) puts these endpoints behind the optional token.
- [Chapter 16](16-rules.md): rules reference stops the same way (by id, no foreign key), and summaries clean their lines with `clean_lines`.
