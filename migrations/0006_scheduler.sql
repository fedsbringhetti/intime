-- Scheduler (Phase 3, step 5): runs of rules, log of notifications, alerts already handled.
CREATE TABLE rule_runs (
    rule_id    INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    date       TEXT NOT NULL,           -- calendar day in Europe/Rome, YYYY-MM-DD (decision AI)
    status     TEXT NOT NULL,           -- no CHECK: step 6 adds statuses without a migration
    due_at     TEXT NOT NULL,
    state      TEXT,                    -- JSON; NULL for summaries
    updated_at TEXT NOT NULL,
    PRIMARY KEY (rule_id, date)
);

CREATE TABLE notifications (
    id      INTEGER PRIMARY KEY,
    rule_id INTEGER REFERENCES rules(id) ON DELETE SET NULL,
    kind    TEXT NOT NULL,
    title   TEXT NOT NULL,
    body    TEXT NOT NULL,
    url     TEXT,
    channel TEXT NOT NULL,
    sent    INTEGER NOT NULL,
    removed INTEGER NOT NULL,
    failed  INTEGER NOT NULL,
    sent_at TEXT NOT NULL
);
CREATE INDEX notifications_sent_at ON notifications (sent_at);

CREATE TABLE notified_alerts (
    alert_id      TEXT PRIMARY KEY,
    first_seen_at TEXT NOT NULL,
    notified_at   TEXT                  -- NULL: not handled yet
);
