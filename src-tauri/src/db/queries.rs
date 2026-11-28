//! The read queries behind the UI.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::library::search::{matches_text, Document};
use crate::progress::{self, Progress};
use crate::tmdb::metadata::{CastMember, Metadata};

#[derive(Debug, Serialize)]
pub struct TitleSummary {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub runtime: Option<u32>,
    pub genres: Vec<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    /// When its newest file was added.
    pub added_at: i64,
    pub file_count: i64,
    /// All of its files have been watched.
    pub watched: bool,
    pub matched: bool,
}

#[derive(Debug, Serialize)]
pub struct EpisodeEntry {
    pub file_id: i64,
    pub season: u32,
    pub episode: u32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    #[serde(flatten)]
    pub file: FileEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub filename: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub position: f64,
    pub duration: Option<f64>,
    pub watched: bool,
    #[serde(skip)]
    updated_at: i64,
}

impl FileEntry {
    fn progress(&self) -> Progress {
        Progress {
            position: self.position,
            duration: self.duration,
            watched: self.watched,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MovieFile {
    pub file_id: i64,
    #[serde(flatten)]
    pub file: FileEntry,
}

#[derive(Debug, Serialize)]
pub struct Season {
    pub number: u32,
    pub name: Option<String>,
    pub episodes: Vec<EpisodeEntry>,
}

/// What the main Play button should do.
#[derive(Debug, Serialize)]
pub struct PlayTarget {
    pub file_id: i64,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub resume_position: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct TitleDetails {
    #[serde(flatten)]
    pub summary: TitleSummary,
    pub tmdb_id: Option<i64>,
    pub tagline: Option<String>,
    pub certification: Option<String>,
    pub directors: Vec<String>,
    pub cast: Vec<CastMember>,
    /// Files of a movie. Empty for shows.
    pub files: Vec<MovieFile>,
    /// Seasons of a show. Empty for movies.
    pub seasons: Vec<Season>,
    pub play: Option<PlayTarget>,
}

#[derive(Debug, Serialize)]
pub struct ContinueItem {
    pub title_id: i64,
    pub file_id: i64,
    pub kind: String,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_name: Option<String>,
    pub position: f64,
    pub duration: Option<f64>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub still_path: Option<String>,
    pub updated_at: i64,
}

/// What the player needs to know about the file it is playing.
#[derive(Debug, Serialize)]
pub struct PlaybackItem {
    pub file_id: i64,
    pub title_id: i64,
    #[serde(skip)]
    pub path: String,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_name: Option<String>,
    /// The local poster file, for the Now Playing widget.
    #[serde(skip)]
    pub poster: Option<String>,
    /// The following episode in season order, if there is one.
    pub next: Option<NextEpisode>,
}

#[derive(Debug, Serialize)]
pub struct NextEpisode {
    pub file_id: i64,
    pub season: u32,
    pub episode: u32,
    pub episode_name: Option<String>,
}

const SUMMARY_COLUMNS: &str = "t.id, t.kind, t.guessed_title, t.guessed_year, t.metadata_json,
     t.poster_path, t.backdrop_path, MAX(f.added_at), COUNT(f.id),
     MIN(COALESCE(p.watched, 0)), t.tmdb_id";

const SUMMARY_FROM: &str = "FROM titles t
     JOIN files f ON f.title_id = t.id AND f.missing = 0
     LEFT JOIN progress p ON p.file_id = f.id";

fn parse_metadata(json: Option<String>) -> Option<Metadata> {
    json.and_then(|json| serde_json::from_str(&json).ok())
}

fn summary_of(row: &Row<'_>) -> rusqlite::Result<(TitleSummary, Option<i64>, Option<Metadata>)> {
    let metadata = parse_metadata(row.get(4)?);
    let meta = metadata.clone().unwrap_or_default();
    let guessed_title: String = row.get(2)?;
    let guessed_year: Option<i64> = row.get(3)?;
    let summary = TitleSummary {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: if meta.title.is_empty() {
            guessed_title
        } else {
            meta.title
        },
        year: meta.year.or(guessed_year),
        rating: meta.rating,
        runtime: meta.runtime,
        genres: meta.genres,
        overview: meta.overview,
        poster_path: row.get(5)?,
        backdrop_path: row.get(6)?,
        added_at: row.get(7)?,
        file_count: row.get(8)?,
        watched: row.get(9)?,
        matched: metadata.is_some(),
    };
    Ok((summary, row.get(10)?, metadata))
}

/// Every title that still has at least one file on disk.
pub fn list_titles(conn: &Connection) -> AppResult<Vec<TitleSummary>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {SUMMARY_COLUMNS} {SUMMARY_FROM} GROUP BY t.id"
    ))?;
    let rows = statement.query_map([], |row| summary_of(row).map(|(summary, _, _)| summary))?;
    let mut titles = rows.collect::<Result<Vec<_>, _>>()?;
    titles.sort_by_cached_key(|t| t.title.to_lowercase());
    Ok(titles)
}

fn episode_entries(conn: &Connection, title_id: i64) -> AppResult<Vec<EpisodeEntry>> {
    // Two copies of one episode may exist; the larger file wins.
    let mut statement = conn.prepare(
        "SELECT f.id, COALESCE(f.season, 0), COALESCE(f.episode, 0), m.name, m.overview,
                m.still_path, f.path, f.size, f.width, f.height,
                p.position, p.duration, p.watched, p.updated_at
         FROM files f
         LEFT JOIN episode_meta m
           ON m.title_id = f.title_id AND m.season = f.season AND m.episode = f.episode
         LEFT JOIN progress p ON p.file_id = f.id
         WHERE f.title_id = ?1 AND f.missing = 0
         ORDER BY f.season, f.episode, f.size DESC, f.path",
    )?;
    let rows = statement.query_map([title_id], |row| {
        Ok(EpisodeEntry {
            file_id: row.get(0)?,
            season: row.get(1)?,
            episode: row.get(2)?,
            name: row.get(3)?,
            overview: row.get(4)?,
            still_path: row.get(5)?,
            file: file_entry(row, 6)?,
        })
    })?;
    let mut episodes = rows.collect::<Result<Vec<_>, _>>()?;
    episodes.dedup_by_key(|e| (e.season, e.episode));
    Ok(episodes)
}

/// Reads path, size, width, height and the four progress columns.
fn file_entry(row: &Row<'_>, at: usize) -> rusqlite::Result<FileEntry> {
    let path: String = row.get(at)?;
    Ok(FileEntry {
        filename: Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(path),
        size: row.get(at + 1)?,
        width: row.get(at + 2)?,
        height: row.get(at + 3)?,
        position: row.get::<_, Option<f64>>(at + 4)?.unwrap_or(0.0),
        duration: row.get(at + 5)?,
        watched: row.get::<_, Option<bool>>(at + 6)?.unwrap_or(false),
        updated_at: row.get::<_, Option<i64>>(at + 7)?.unwrap_or(0),
    })
}

fn movie_files(conn: &Connection, title_id: i64) -> AppResult<Vec<MovieFile>> {
    let mut statement = conn.prepare(
        "SELECT f.id, f.path, f.size, f.width, f.height,
                p.position, p.duration, p.watched, p.updated_at
         FROM files f LEFT JOIN progress p ON p.file_id = f.id
         WHERE f.title_id = ?1 AND f.missing = 0
         ORDER BY COALESCE(p.updated_at, 0) DESC, f.size DESC, f.path",
    )?;
    let rows = statement.query_map([title_id], |row| {
        Ok(MovieFile {
            file_id: row.get(0)?,
            file: file_entry(row, 1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn next_episode_index(episodes: &[EpisodeEntry]) -> Option<usize> {
    let states: Vec<_> = episodes
        .iter()
        .map(|e| (e.season, e.file.progress()))
        .collect();
    progress::next_up(&states)
}

fn episode_target(episode: &EpisodeEntry) -> PlayTarget {
    PlayTarget {
        file_id: episode.file_id,
        season: Some(episode.season),
        episode: Some(episode.episode),
        resume_position: progress::resume_position(episode.file.position, episode.file.duration),
    }
}

pub fn title_details(conn: &Connection, id: i64) -> AppResult<TitleDetails> {
    let (summary, tmdb_id, metadata) = conn
        .query_row(
            &format!("SELECT {SUMMARY_COLUMNS} {SUMMARY_FROM} WHERE t.id = ?1 GROUP BY t.id"),
            [id],
            summary_of,
        )
        .optional()?
        .ok_or(AppError::NotFound)?;
    let metadata = metadata.unwrap_or_default();

    let mut files = Vec::new();
    let mut seasons = Vec::new();
    let play;
    if summary.kind == "movie" {
        files = movie_files(conn, id)?;
        play = files.first().map(|f| PlayTarget {
            file_id: f.file_id,
            season: None,
            episode: None,
            resume_position: progress::resume_position(f.file.position, f.file.duration),
        });
    } else {
        let episodes = episode_entries(conn, id)?;
        // A finished show offers to start over from the first episode.
        play = next_episode_index(&episodes)
            .or((!episodes.is_empty()).then_some(0))
            .map(|index| episode_target(&episodes[index]));
        let names: HashMap<u32, Option<String>> = metadata
            .seasons
            .iter()
            .map(|s| (s.number, s.name.clone()))
            .collect();
        let mut by_season: BTreeMap<u32, Vec<EpisodeEntry>> = BTreeMap::new();
        for episode in episodes {
            by_season.entry(episode.season).or_default().push(episode);
        }
        seasons = by_season
            .into_iter()
            .map(|(number, episodes)| Season {
                number,
                name: names.get(&number).cloned().flatten(),
                episodes,
            })
            .collect();
    }

    Ok(TitleDetails {
        summary,
        tmdb_id,
        tagline: metadata.tagline,
        certification: metadata.certification,
        directors: metadata.directors,
        cast: metadata.cast,
        files,
        seasons,
        play,
    })
}

/// Movies in progress, and for each started show the episode to watch next.
/// Newest activity first.
pub fn continue_watching(conn: &Connection) -> AppResult<Vec<ContinueItem>> {
    let mut statement = conn.prepare(
        "SELECT f.title_id, MAX(p.updated_at) FROM progress p
         JOIN files f ON f.id = p.file_id AND f.missing = 0
         GROUP BY f.title_id ORDER BY MAX(p.updated_at) DESC",
    )?;
    let active = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<(i64, i64)>, _>>()?;

    let mut items = Vec::new();
    for (title_id, updated_at) in active {
        let details = title_details(conn, title_id)?;
        let summary = details.summary;
        let item = |file_id, file: &FileEntry| ContinueItem {
            title_id,
            file_id,
            kind: summary.kind.clone(),
            title: summary.title.clone(),
            season: None,
            episode: None,
            episode_name: None,
            position: progress::resume_position(file.position, file.duration).unwrap_or(0.0),
            duration: file.duration,
            poster_path: summary.poster_path.clone(),
            backdrop_path: summary.backdrop_path.clone(),
            still_path: None,
            updated_at,
        };
        if summary.kind == "movie" {
            let in_progress = details
                .files
                .iter()
                .find(|f| progress::resume_position(f.file.position, f.file.duration).is_some());
            items.extend(in_progress.map(|f| item(f.file_id, &f.file)));
        } else {
            let episodes: Vec<EpisodeEntry> = details
                .seasons
                .into_iter()
                .flat_map(|s| s.episodes)
                .collect();
            items.extend(next_episode_index(&episodes).map(|index| {
                let e = &episodes[index];
                ContinueItem {
                    season: Some(e.season),
                    episode: Some(e.episode),
                    episode_name: e.name.clone(),
                    still_path: e.still_path.clone(),
                    ..item(e.file_id, &e.file)
                }
            }));
        }
    }
    Ok(items)
}

pub fn playback_item(conn: &Connection, file_id: i64) -> AppResult<PlaybackItem> {
    let (title_id, path): (i64, String) = conn
        .query_row(
            "SELECT title_id, path FROM files WHERE id = ?1 AND missing = 0",
            [file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(AppError::NotFound)?;
    let details = title_details(conn, title_id)?;
    let mut item = PlaybackItem {
        file_id,
        title_id,
        path,
        title: details.summary.title,
        poster: details.summary.poster_path,
        season: None,
        episode: None,
        episode_name: None,
        next: None,
    };

    let current: Option<(Option<u32>, Option<u32>)> = conn
        .query_row(
            "SELECT season, episode FROM files WHERE id = ?1",
            [file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((Some(season), Some(episode))) = current {
        let episodes: Vec<EpisodeEntry> = details
            .seasons
            .into_iter()
            .flat_map(|s| s.episodes)
            .collect();
        item.season = Some(season);
        item.episode = Some(episode);
        item.episode_name = episodes
            .iter()
            .find(|e| (e.season, e.episode) == (season, episode))
            .and_then(|e| e.name.clone());
        item.next = episodes
            .into_iter()
            .find(|e| (e.season, e.episode) > (season, episode))
            .map(|e| NextEpisode {
                file_id: e.file_id,
                season: e.season,
                episode: e.episode,
                episode_name: e.name,
            });
    }
    Ok(item)
}

pub fn file_path(conn: &Connection, file_id: i64) -> AppResult<String> {
    conn.query_row("SELECT path FROM files WHERE id = ?1", [file_id], |row| {
        row.get(0)
    })
    .optional()?
    .ok_or(AppError::NotFound)
}

pub fn title_file_ids(conn: &Connection, title_id: i64) -> AppResult<Vec<i64>> {
    let mut statement = conn.prepare("SELECT id FROM files WHERE title_id = ?1 AND missing = 0")?;
    let rows = statement.query_map([title_id], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

const EPISODE_RESULTS: usize = 12;

/// Episodes on disk whose name matches the query, ready to be played.
pub fn search_episodes(conn: &Connection, query: &str) -> AppResult<Vec<ContinueItem>> {
    let mut statement = conn.prepare(
        "SELECT m.title_id, m.season, m.episode, m.name FROM episode_meta m
         JOIN titles t ON t.id = m.title_id
         WHERE m.name IS NOT NULL
         ORDER BY t.guessed_title COLLATE NOCASE, m.season, m.episode",
    )?;
    let named = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<(i64, u32, u32, String)>, _>>()?;

    let mut shows: HashMap<i64, TitleDetails> = HashMap::new();
    let mut hits = Vec::new();
    for (title_id, season, number, name) in named {
        if hits.len() == EPISODE_RESULTS {
            break;
        }
        if !matches_text(query, &name) {
            continue;
        }
        let details = match shows.entry(title_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            // A show with no files left on disk has no details.
            Entry::Vacant(entry) => match title_details(conn, title_id) {
                Ok(details) => entry.insert(details),
                Err(AppError::NotFound) => continue,
                Err(error) => return Err(error),
            },
        };
        let found = details
            .seasons
            .iter()
            .flat_map(|s| &s.episodes)
            .find(|e| (e.season, e.episode) == (season, number));
        if let Some(episode) = found {
            let file = &episode.file;
            hits.push(ContinueItem {
                title_id,
                file_id: episode.file_id,
                kind: details.summary.kind.clone(),
                title: details.summary.title.clone(),
                season: Some(season),
                episode: Some(number),
                episode_name: episode.name.clone(),
                position: progress::resume_position(file.position, file.duration).unwrap_or(0.0),
                duration: file.duration,
                poster_path: details.summary.poster_path.clone(),
                backdrop_path: details.summary.backdrop_path.clone(),
                still_path: episode.still_path.clone(),
                updated_at: file.updated_at,
            });
        }
    }
    Ok(hits)
}

pub fn search_documents(conn: &Connection) -> AppResult<Vec<Document>> {
    let mut episode_names: HashMap<i64, Vec<String>> = HashMap::new();
    let mut statement = conn.prepare(
        "SELECT m.title_id, m.name FROM episode_meta m
         JOIN files f ON f.title_id = m.title_id AND f.season = m.season
              AND f.episode = m.episode AND f.missing = 0
         WHERE m.name IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    for row in rows {
        let (title_id, name) = row?;
        episode_names.entry(title_id).or_default().push(name);
    }

    let mut statement = conn.prepare(&format!(
        "SELECT {SUMMARY_COLUMNS} {SUMMARY_FROM} GROUP BY t.id"
    ))?;
    let rows = statement.query_map([], summary_of)?;
    let mut documents = Vec::new();
    for row in rows {
        let (summary, _, metadata) = row?;
        let mut extra: Vec<String> = metadata
            .unwrap_or_default()
            .cast
            .into_iter()
            .map(|member| member.name)
            .collect();
        extra.extend(episode_names.remove(&summary.id).unwrap_or_default());
        documents.push(Document {
            id: summary.id,
            title: summary.title,
            extra,
        });
    }
    Ok(documents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn library() -> Connection {
        let conn = db::open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO titles (id, kind, name_key, guessed_title) VALUES
               (1, 'show', 'dark', 'Dark'), (2, 'movie', 'heat', 'Heat');
             INSERT INTO files (id, path, title_id, season, episode, size, mtime, added_at) VALUES
               (10, '/m/dark.s01e01.mkv', 1, 1, 1, 5, 0, 1),
               (11, '/m/dark.s01e02.mkv', 1, 1, 2, 5, 0, 1),
               (12, '/m/dark.s02e01.mkv', 1, 2, 1, 5, 0, 1),
               (20, '/m/heat.mkv', 2, NULL, NULL, 5, 0, 2);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_finished_episode_moves_on_across_seasons() {
        let conn = library();
        progress::set_watched(&conn, 10, true, 1).unwrap();
        progress::set_watched(&conn, 11, true, 2).unwrap();
        progress::save(&conn, 20, 600.0, Some(6000.0), 3).unwrap();

        let items = continue_watching(&conn).unwrap();
        let ids: Vec<_> = items.iter().map(|i| i.file_id).collect();
        assert_eq!(ids, [20, 12]);
        assert!((items[0].position - 600.0).abs() < f64::EPSILON);

        let playing = playback_item(&conn, 11).unwrap();
        assert_eq!(playing.next.map(|n| n.file_id), Some(12));
        assert_eq!(title_details(&conn, 1).unwrap().seasons.len(), 2);
    }

    #[test]
    fn episode_search_only_returns_files_on_disk() {
        let conn = library();
        conn.execute_batch(
            "INSERT INTO episode_meta (title_id, season, episode, name) VALUES
               (1, 1, 2, 'Lies'), (1, 9, 9, 'Lies, gone'), (1, 2, 1, 'Beginnings');",
        )
        .unwrap();
        let hits = search_episodes(&conn, "lie").unwrap();
        assert_eq!(hits.iter().map(|h| h.file_id).collect::<Vec<_>>(), [11]);
        assert!(search_episodes(&conn, "  ").unwrap().is_empty());
    }

    #[test]
    fn watched_means_every_file() {
        let conn = library();
        progress::set_watched(&conn, 10, true, 1).unwrap();
        let titles = list_titles(&conn).unwrap();
        assert!(!titles[0].watched);
        assert_eq!(titles[0].file_count, 3);
    }
}
