-- Trip tracking (Phase 3, step 6).

-- One row per tracked run, shared by every rule tracking it (decision AY).
CREATE TABLE observed_delays (
    service_date   TEXT NOT NULL,
    trip_id        TEXT NOT NULL,
    stop_id        TEXT NOT NULL,
    line           TEXT NOT NULL,
    scheduled      TEXT NOT NULL,
    final_expected TEXT NOT NULL,
    delay_seconds  INTEGER NOT NULL,
    source         TEXT NOT NULL,        -- where the final time came from: gtt, otp, scheduled
    PRIMARY KEY (service_date, trip_id, stop_id)
);

-- Alerts already sent, per rule (decision AZ). The baseline in notified_alerts stays global.
CREATE TABLE rule_notified_alerts (
    rule_id     INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    alert_id    TEXT NOT NULL,
    notified_at TEXT NOT NULL,
    PRIMARY KEY (rule_id, alert_id)
);
