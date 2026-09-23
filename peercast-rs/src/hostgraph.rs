//! リレーツリー (core/common/hostgraph.cpp の `HostGraph` のコンストラクター)。
//!
//! どのホストをどのホストの下に置くかを決める。JSON を組み立てる `toRelayTree` と
//! `getRelayTree` は C++ 側。

use crate::chanhit::Host;
use std::cmp::Ordering;

/// `ChanHit` のうち、ここで使う欄
#[derive(Clone, Copy, Debug, Default)]
pub struct Node {
    pub rhost: [Host; 2],
    pub uphost: Host,
}

/// `Host::operator<` (IP のバイト列、同じならポート)
fn cmp_host(a: &Host, b: &Host) -> Ordering {
    a.ip.cmp(&b.ip).then(a.port.cmp(&b.port))
}

/// `HostGraph::ID` (`rhost[0]`, `rhost[1]`) の `std::pair` の順序
fn cmp_id(a: &Node, b: &Node) -> Ordering {
    cmp_host(&a.rhost[0], &b.rhost[0]).then_with(|| cmp_host(&a.rhost[1], &b.rhost[1]))
}

/// `Host()` (`::ffff:0.0.0.0` の 0 番ポート)
const NO_HOST: Host = Host { ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0], port: 0 };

/// `std::map<ID, ChanHit>` の 1 つ分
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// 採った `ChanHit` の番号 (同じ ID が何度も出たら最後のもの)
    pub index: usize,
    /// 親の `Entry` の番号。なければ根 (`m_roots`)。
    pub parent: Option<usize>,
}

/// `HostGraph::HostGraph`。`nodes` は自分 (0 番) のあとにリストの順で並べたもの。
/// 返り値は ID の順 (`std::map` を回す順)。`m_roots` と `m_children[x]` の中身は、どれも
/// この順に並ぶ。
pub fn build(nodes: &[Node]) -> Vec<Entry> {
    // m_hit[id(*p)] = *p: ID の順に並べ、同じ ID は後のものが勝つ
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| cmp_id(&nodes[a], &nodes[b]).then(b.cmp(&a)));
    order.dedup_by(|a, b| cmp_id(&nodes[*a], &nodes[*b]) == Ordering::Equal);

    let ids: Vec<&Node> = order.iter().map(|&i| &nodes[i]).collect();
    order
        .iter()
        .map(|&i| {
            let hit = &nodes[i];
            let parent = if hit.uphost == NO_HOST {
                // tracker
                None
            } else {
                // wan relay (fetch)
                ids.iter()
                    .position(|id1| id1.rhost[0] == hit.uphost)
                    // wan relay (push)
                    .or_else(|| ids.iter().position(|id1| id1.rhost[0].ip == hit.uphost.ip))
                    // lan relay (fetch)
                    .or_else(|| {
                        ids.iter()
                            .position(|id1| id1.rhost[0].ip == hit.rhost[0].ip && id1.rhost[1] == hit.uphost)
                    })
            };
            Entry { index: i, parent }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(a: u8, port: u16) -> Host {
        Host { ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 10, 0, 0, a], port }
    }

    fn node(r0: Host, r1: Host, up: Host) -> Node {
        Node { rhost: [r0, r1], uphost: up }
    }

    #[test]
    fn tracker_and_children() {
        let nodes = [
            node(h(1, 7144), h(1, 7144), NO_HOST),
            node(h(3, 7144), h(3, 7144), h(1, 7144)),
            node(h(2, 7144), h(2, 7144), h(1, 7144)),
            node(h(4, 7144), h(4, 7144), h(2, 1)), // push: IP だけ一致
        ];
        let g = build(&nodes);
        assert_eq!(
            g,
            vec![
                Entry { index: 0, parent: None },
                Entry { index: 2, parent: Some(0) },
                Entry { index: 1, parent: Some(0) },
                Entry { index: 3, parent: Some(1) },
            ]
        );
    }

    #[test]
    fn duplicate_id_last_wins_and_unknown_uphost_is_root() {
        let nodes = [
            node(h(1, 1), h(1, 1), h(9, 9)),
            node(h(1, 1), h(1, 1), NO_HOST),
            node(h(5, 1), h(6, 2), h(7, 7)),
        ];
        let g = build(&nodes);
        assert_eq!(g, vec![Entry { index: 1, parent: None }, Entry { index: 2, parent: None }]);
    }

    #[test]
    fn lan_relay_and_self_parent() {
        let nodes = [
            node(h(1, 1), h(20, 5), h(30, 1)),
            node(h(1, 2), h(21, 5), h(20, 5)), // LAN: rhost[0].ip 同じで、rhost[1] が uphost
            node(h(8, 8), h(8, 8), h(8, 8)),   // 自分自身が親
        ];
        let g = build(&nodes);
        assert_eq!(
            g,
            vec![
                Entry { index: 0, parent: None },
                Entry { index: 1, parent: Some(0) },
                Entry { index: 2, parent: Some(2) },
            ]
        );
    }
}
