#!/bin/sh
# Crea/puebla una BD SQLite para probar db-monitor (tabla `events` del config de ejemplo).
#   scripts/sqlite-demo.sh [db] [filas]          crea si falta e inserta N filas (default 10)
#   scripts/sqlite-demo.sh [db] [filas] [secs]   además sigue insertando N filas cada secs (Ctrl-C para parar)
set -eu
DB=${1:-demo.db}
N=${2:-10}
EVERY=${3:-}

sqlite3 "$DB" "CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY,
  message TEXT,
  created_at TEXT DEFAULT CURRENT_TIMESTAMP);"

insert() {
  sqlite3 "$DB" "WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM s WHERE i < $N)
    INSERT INTO events(message) SELECT 'evento ' || abs(random() % 10000) FROM s;
    SELECT 'filas: ' || count(*) || ', max id: ' || max(id) FROM events;"
}

insert
[ -z "$EVERY" ] && exit 0
while sleep "$EVERY"; do insert; done
