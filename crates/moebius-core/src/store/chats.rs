//! Chats: runs a person talks to, with where they were started, what they work in and how far a person has read them.

use rusqlite::params;

use super::{Result, Run, Store};

impl Store {
    /// The chats, newest first.
    pub fn chats(&self) -> Result<Vec<Run>> {
        let mut chats = self.runs_where("purpose = ?1", "chat".to_string().into())?;
        chats.reverse();
        Ok(chats)
    }

    /// How far a person has read the run: the `seq` of the last of its events they have seen.
    pub fn seen_seq(&self, run_id: i64) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT seen_seq FROM runs WHERE id = ?1", [run_id], |r| {
                r.get(0)
            })?)
    }

    /// A person has read the run up to the event `seq`; reading never goes back.
    pub fn set_seen_seq(&self, run_id: i64, seq: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET seen_seq = MAX(seen_seq, ?2) WHERE id = ?1",
            params![run_id, seq],
        )?;
        Ok(())
    }

    /// The `seq` of the last event of the run that a person has to read: an answer that settled, or an error. Zero
    /// without one.
    pub fn answered_seq(&self, run_id: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM events
             WHERE run_id = ?1 AND kind IN ('agent_settled', 'moebius_error')",
            [run_id],
            |r| r.get(0),
        )?)
    }

    /// Marks a waiting chat as running; a chat in any other state stays as it is.
    pub fn set_chat_working(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET status = 'running' WHERE id = ?1 AND status = 'waiting'",
            [id],
        )?;
        Ok(())
    }

    /// Picks up a chat that ended when its agent went away: it works from now on and is no longer ended, so that what
    /// the person writes next belongs to it again.
    pub fn reopen_chat(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET status = 'running', ended_at = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    pub fn set_run_repository(&self, id: i64, repository_id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET repository_id = ?1 WHERE id = ?2",
            params![repository_id, id],
        )?;
        Ok(())
    }

    /// Records where the chat was started (see `Run::origin`).
    pub fn set_run_origin(&self, id: i64, origin: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET origin = ?1 WHERE id = ?2",
            params![origin, id],
        )?;
        Ok(())
    }

    /// Records that the chat works in the harness configuration repository.
    pub fn set_run_harness_repository(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("UPDATE runs SET harness_repository = 1 WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::store::{MIGRATIONS, Store};

    #[test]
    fn a_chat_keeps_where_it_was_started_and_one_from_before_was_started_on_the_chat_page() {
        let dir = tempfile::tempdir().unwrap();
        let before = MIGRATIONS.len() - 1;
        {
            let conn = Connection::open(dir.path().join("moebius.sqlite")).unwrap();
            for sql in &MIGRATIONS[..before] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", before as i64)
                .unwrap();
            conn.execute(
                "INSERT INTO runs (agent, harness, purpose, status, started_at) VALUES ('Moebius', 'pi-bolt', 'chat', 'waiting', 1)",
                [],
            )
            .unwrap();
        }
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.run(1).unwrap().unwrap().origin, "chat");
        let run = s.add_run(None, "Moebius", "pi-bolt", "chat").unwrap();
        s.set_run_origin(run.id, "assistant").unwrap();
        let origins: Vec<String> = s.chats().unwrap().into_iter().map(|r| r.origin).collect();
        assert_eq!(origins, ["assistant", "chat"]);
    }
}
