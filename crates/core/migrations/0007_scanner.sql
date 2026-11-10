-- What the scanner read from each file name, for the "unmatched" list and re-matching.
ALTER TABLE local_file ADD COLUMN parsed_title TEXT;
ALTER TABLE local_file ADD COLUMN parsed_season INTEGER;
-- Files the user said aren't episodes.
ALTER TABLE local_file ADD COLUMN ignored INTEGER NOT NULL DEFAULT 0;

-- Cached AniList search results per parsed title (media_id NULL = no confident match).
CREATE TABLE title_match (
    query       TEXT PRIMARY KEY,
    media_id    INTEGER,
    checked_at  TEXT NOT NULL
);
