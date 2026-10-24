//! A3 D4: the player store for per-player revenue and test group.
//!
//! Ids are resolved in one order: User ID, IDFV, IDFA. A zeroed IDFA is
//! missing, never a player called "00000000-0000-0000-0000-000000000000".

use std::collections::BTreeMap;

use apex_core::Micros;

/// One player-day observation from an impression or revenue feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerDay {
    pub day: String,
    pub user_id: Option<String>,
    pub idfv: Option<String>,
    pub idfa: Option<String>,
    pub test: bool,
    pub revenue_micros: Micros,
}

/// The id chosen for a row, or `None` when every id is missing.
#[must_use]
pub fn player_key(row: &PlayerDay) -> Option<String> {
    fn valid(id: &Option<String>) -> Option<&str> {
        id.as_deref().filter(|value| {
            !value.is_empty()
                && *value != "00000000-0000-0000-0000-000000000000"
                && *value != "00000000000000000000000000000000"
        })
    }
    valid(&row.user_id)
        .or_else(|| valid(&row.idfv))
        .or_else(|| valid(&row.idfa))
        .map(str::to_string)
}

/// Sum revenue per resolved player. Rows with no usable id are returned
/// separately so the caller can report the uncovered revenue share.
#[must_use]
pub fn player_totals(rows: &[PlayerDay]) -> (BTreeMap<String, Micros>, Micros) {
    let mut players = BTreeMap::new();
    let mut unidentified = 0;
    for row in rows {
        match player_key(row) {
            Some(key) => *players.entry(key).or_default() += row.revenue_micros,
            None => unidentified += row.revenue_micros,
        }
    }
    (players, unidentified)
}

/// The share of revenue with no id in basis points.
#[must_use]
pub fn unidentified_share_bp(unidentified: Micros, total: Micros) -> i64 {
    if total <= 0 || unidentified <= 0 {
        return 0;
    }
    ((i128::from(unidentified) * 10_000) / i128::from(total)).min(10_000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        user_id: Option<&str>,
        idfv: Option<&str>,
        idfa: Option<&str>,
        revenue: i64,
    ) -> PlayerDay {
        PlayerDay {
            day: "2026-10-01".into(),
            user_id: user_id.map(str::to_string),
            idfv: idfv.map(str::to_string),
            idfa: idfa.map(str::to_string),
            test: true,
            revenue_micros: revenue,
        }
    }

    #[test]
    fn ids_fall_back_in_the_documented_order() {
        assert_eq!(
            player_key(&row(Some("u"), Some("v"), Some("a"), 1)),
            Some("u".into())
        );
        assert_eq!(
            player_key(&row(None, Some("v"), Some("a"), 1)),
            Some("v".into())
        );
        assert_eq!(player_key(&row(None, None, Some("a"), 1)), Some("a".into()));
        assert_eq!(
            player_key(&row(
                None,
                None,
                Some("00000000-0000-0000-0000-000000000000"),
                1
            )),
            None
        );
    }

    #[test]
    fn revenue_sums_per_player_and_reports_the_uncovered_share() {
        let rows = vec![
            row(Some("u1"), None, None, 100),
            row(Some("u1"), None, None, 50),
            row(None, None, None, 50),
        ];
        let (players, unidentified) = player_totals(&rows);
        assert_eq!(players.get("u1"), Some(&150));
        assert_eq!(unidentified, 50);
        assert_eq!(unidentified_share_bp(50, 200), 2_500);
    }
}
