//! 模型测速队列与 worker。界面之外的后台服务：
//! 窗口关闭后测速继续跑完，结果持久化进 model_cache。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock, PoisonError, RwLock};

use crate::logic::config::AiConfig;
use crate::logic::model_cache;

/// 同一个端点上同时在飞的探测数（用户 2026-09-17 定的，旧版 `latency.js` 是 1）。
/// 旧版压到 1 是因为反代限流回的 429 和「模型真不能用」长得一模一样，会测出一片假失败；
/// 新版多了一层兜底 —— 429 / 5xx / 超时会等 2 秒重试一次（`service::ai::probe_latency`），
/// 假失败大概率捞得回来。**要再调就只改这一个数**，别动 worker 的循环。
const CONCURRENCY: usize = 3;
/// 同一个 worker 两次探测之间喘口气，同样是为了不触发限流（旧版 `GAP`）。
const GAP_MS: u64 = 300;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub running: bool,
    /// 正在飞的第一个模型名（给折叠条显示用），没有就空串
    pub current: String,
    /// 正在飞的全部模型名（每行的「测速中」靠它判断）
    pub testing: Vec<String>,
    /// 还在排队的个数（不含正在飞的那些）
    pub remaining: u32,
}

struct Queue {
    pending: VecDeque<String>,
    /// 正在飞的模型，最多 `CONCURRENCY` 个
    testing: Vec<String>,
    /// 还活着的 worker 线程数；归零时这一项从表里删掉
    workers: usize,
    config: AiConfig,
}

type Notifier = Option<Box<dyn Fn() + Send + Sync>>;

static QUEUES: OnceLock<Mutex<HashMap<String, Queue>>> = OnceLock::new();
static NOTIFIER: OnceLock<RwLock<Notifier>> = OnceLock::new();

fn queues() -> &'static Mutex<HashMap<String, Queue>> {
    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn notify() {
    if let Some(holder) = NOTIFIER.get() {
        let guard = holder.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(ref cb) = *guard {
            cb();
        }
    }
}

/// 每完成一个模型（以及 start / stop 改了状态）调一次，让界面去重新读状态。
/// 后注册的覆盖先注册的；没注册就什么也不做。
pub fn set_notifier(f: Box<dyn Fn() + Send + Sync>) {
    let holder = NOTIFIER.get_or_init(|| RwLock::new(None));
    let mut guard = holder.write().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(f);
}

/// 纯函数：从候选列表中挑出待测的模型。
pub(crate) fn pick_pending(
    models: &[String],
    latencies: &BTreeMap<String, Option<u32>>,
    force: bool,
    busy: &[String],
) -> Vec<String> {
    models
        .iter()
        .filter(|m| {
            let name = m.as_str();
            if busy.iter().any(|b| b == name) {
                return false;
            }
            if force {
                return true;
            }
            !matches!(latencies.get(name), Some(Some(_)))
        })
        .cloned()
        .collect()
}

/// 纯函数：已有 `existing` 个 worker、队里还剩 `pending` 个待测时，还要再起几个。
/// 封顶 `CONCURRENCY`，而且不会为了凑数起一个没活干的 worker。
fn workers_to_spawn(existing: usize, pending: usize) -> usize {
    CONCURRENCY.min(existing + pending).saturating_sub(existing)
}

/// 把这些模型排进这个端点的队列并保证有 worker 在跑。
/// `force = true`：先抹掉这些模型的旧结果，全部重测；
/// `force = false`：只挑「还没有数值结果的」（失败过的会被重测，照旧版）。
pub fn start(config: &AiConfig, models: Vec<String>, force: bool) {
    let endpoint = model_cache::endpoint_of(&config.base_url, &config.protocol);
    if endpoint.is_empty() {
        return;
    }

    if force {
        model_cache::clear_latencies(&endpoint, &models);
    }

    let latencies = model_cache::latencies(&endpoint);

    // 要补几个 worker：已有的加上待测的，封顶 CONCURRENCY。
    let spawn_count = {
        let queues = queues();
        let mut guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
        let q = guard.entry(endpoint.clone()).or_insert_with(|| Queue {
            pending: VecDeque::new(),
            testing: Vec::new(),
            workers: 0,
            config: config.clone(),
        });
        // 用户可能刚改了 key / 地址，后续探测用最新这份
        q.config = config.clone();

        let mut busy = q.testing.clone();
        busy.extend(q.pending.iter().cloned());
        q.pending
            .extend(pick_pending(&models, &latencies, force, &busy));

        let n = workers_to_spawn(q.workers, q.pending.len());
        q.workers += n;
        // 没东西可测又没人在跑：把刚才 entry() 建出来的空壳清掉，别让 progress 看见一个假队列
        let idle = q.pending.is_empty() && q.testing.is_empty() && q.workers == 0;
        if idle {
            guard.remove(&endpoint);
        }
        n
    };

    for _ in 0..spawn_count {
        let endpoint_clone = endpoint.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("benchmark".into())
            .spawn(move || {
                run_worker(endpoint_clone);
            })
        {
            log::error!("benchmark: failed to spawn worker thread: {e}");
            let queues = queues();
            let mut guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(q) = guard.get_mut(&endpoint) {
                q.workers -= 1;
                if q.workers == 0 && q.testing.is_empty() {
                    guard.remove(&endpoint);
                }
            }
        }
    }

    notify();
}

/// 只丢还在排队的。正在飞的那一发拦不住，让 worker 自己收尾。
pub fn stop(endpoint: &str) {
    if endpoint.is_empty() {
        return;
    }
    {
        let queues = queues();
        let mut guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(q) = guard.get_mut(endpoint) {
            q.pending.clear();
        }
    }
    notify();
}

pub fn progress(endpoint: &str) -> Progress {
    if endpoint.is_empty() {
        return Progress::default();
    }
    let queues = queues();
    let guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
    match guard.get(endpoint) {
        Some(q) => Progress {
            running: !q.testing.is_empty() || !q.pending.is_empty(),
            current: q.testing.first().cloned().unwrap_or_default(),
            testing: q.testing.clone(),
            remaining: q.pending.len() as u32,
        },
        None => Progress::default(),
    }
}

fn run_worker(endpoint: String) {
    loop {
        // 1. 上锁：从 pending 取一个放进 testing；取不到就退场
        //    （最后一个 worker 走的时候才把队列从表里删掉）
        let (model, config) = {
            let queues = queues();
            let mut guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
            match guard.get_mut(&endpoint) {
                Some(q) => match q.pending.pop_front() {
                    Some(m) => {
                        q.testing.push(m.clone());
                        (Some(m), q.config.clone())
                    }
                    None => {
                        q.workers -= 1;
                        let done = q.workers == 0 && q.testing.is_empty();
                        if done {
                            guard.remove(&endpoint);
                        }
                        (None, AiConfig::default())
                    }
                },
                None => (None, AiConfig::default()),
            }
        };

        let Some(model) = model else {
            notify();
            break;
        };

        // 2. notify（界面这时候要显示「测速中 <模型名>」）
        notify();

        // 3. 锁外执行网络探测
        let probe_res = crate::service::ai::probe_latency(&config, &model);
        let ms = match probe_res {
            Ok(elapsed) => Some(elapsed),
            Err(e) => {
                log::debug!("benchmark: probe failed for {model}: {e}");
                None
            }
        };
        model_cache::record_latency(&endpoint, &model, ms);

        // 4. 上锁把自己这一发从 testing 里摘掉，判断还有没有待测项
        let has_more = {
            let queues = queues();
            let mut guard = queues.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(q) = guard.get_mut(&endpoint) {
                q.testing.retain(|m| m != &model);
                !q.pending.is_empty()
            } else {
                false
            }
        };

        notify();

        // 5. 还有待测项时喘一口再进下一轮
        if has_more {
            std::thread::sleep(std::time::Duration::from_millis(GAP_MS));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_pending_filtering() {
        let models = vec![
            "model-tested-ok".to_string(),
            "model-tested-failed".to_string(),
            "model-untested".to_string(),
            "model-busy".to_string(),
        ];

        let mut latencies = BTreeMap::new();
        latencies.insert("model-tested-ok".to_string(), Some(120));
        latencies.insert("model-tested-failed".to_string(), None);

        let busy = vec!["model-busy".to_string()];

        // 1. non-force: 跳过已有数值的、跳过 busy 的，保留失败的 (None) 和未测的
        let picked = pick_pending(&models, &latencies, false, &busy);
        assert_eq!(picked, vec!["model-tested-failed", "model-untested"]);

        // 2. force: 保留所有非 busy 的模型（包括已有数值的）
        let picked_force = pick_pending(&models, &latencies, true, &busy);
        assert_eq!(
            picked_force,
            vec!["model-tested-ok", "model-tested-failed", "model-untested"]
        );

        // 3. 全部都在 busy 中时，结果为空
        let all_busy = vec![
            "model-tested-ok".to_string(),
            "model-tested-failed".to_string(),
            "model-untested".to_string(),
            "model-busy".to_string(),
        ];
        assert!(pick_pending(&models, &latencies, true, &all_busy).is_empty());
    }

    #[test]
    fn workers_to_spawn_caps_at_concurrency() {
        // 空队列起步：待测多少就起多少，封顶 CONCURRENCY
        assert_eq!(workers_to_spawn(0, 0), 0);
        assert_eq!(workers_to_spawn(0, 1), 1);
        assert_eq!(workers_to_spawn(0, CONCURRENCY), CONCURRENCY);
        assert_eq!(workers_to_spawn(0, CONCURRENCY + 10), CONCURRENCY);
        // 已经满员就不再起
        assert_eq!(workers_to_spawn(CONCURRENCY, 50), 0);
        // 有 worker 退场后再排队：补到满员为止
        assert_eq!(workers_to_spawn(1, 50), CONCURRENCY - 1);
        // 只剩一个待测就只补一个，不凑数
        assert_eq!(workers_to_spawn(1, 1), 1);
    }

    #[test]
    fn progress_empty_endpoint() {
        let p = progress("");
        assert!(!p.running);
        assert_eq!(p.current, "");
        assert_eq!(p.remaining, 0);
    }
}
