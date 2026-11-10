-- What AniList knows about the next episode, so "Up next" only offers aired episodes.
ALTER TABLE media ADD COLUMN next_episode INTEGER;
ALTER TABLE media ADD COLUMN next_airing_at TEXT;
