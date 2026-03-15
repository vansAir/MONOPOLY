use axum::{
    extract::{Query, State},
    http::Method,
    routing::get,
    Json, Router,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

// =====================================================================
// ===== 共享状态定义 =====
// =====================================================================

// 玩家名 -> 金币数 (单位: K)
type Players = Arc<DashMap<String, i64>>;

// =====================================================================
// ===== 请求参数结构体 (Query Params) =====
// =====================================================================

#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

#[derive(Deserialize)]
struct TransferQuery {
    from: String,
    to: String,
    amount: i64,
}

#[derive(Deserialize)]
struct PayBankQuery {
    name: String,
    amount: i64,
}

#[derive(Deserialize)]
struct AdminPasswordQuery {
    password: String,
}

#[derive(Deserialize)]
struct AdminAddQuery {
    password: String,
    name: String,
    amount: i64,
}

#[derive(Deserialize)]
struct AdminGiveAllQuery {
    password: String,
    amount: i64,
}

#[derive(Deserialize)]
struct AdminDeleteQuery {
    password: String,
    names: String,
}

// =====================================================================
// ===== 工具函数 =====
// =====================================================================

const ADMIN_PASSWORD: &str = "1874";

/// 格式化金币显示，例如 1500 -> 1M 500K
fn format_coins(coins: i64) -> String {
    if coins >= 1000 {
        let m = coins / 1000;
        let k = coins % 1000;
        if k != 0 {
            format!("{}M {}K", m, k)
        } else {
            format!("{}M", m)
        }
    } else {
        format!("{}K", coins)
    }
}

/// 校验管理员权限
fn check_admin(password: &str) -> Option<Value> {
    if password != ADMIN_PASSWORD {
        Some(json!({ "success": false, "msg": "密码错误" }))
    } else {
        None
    }
}

// =====================================================================
// ===== 玩家业务接口 (API Handlers) =====
// =====================================================================

/// GET /api/login?name=xxx
/// 登录或注册新玩家
async fn login(State(players): State<Players>, Query(q): Query<NameQuery>) -> Json<Value> {
    let name = q.name.trim().to_string();
    if name.is_empty() {
        return Json(json!({ "success": false, "msg": "昵称不能为空" }));
    }

    let is_new = !players.contains_key(&name);
    if is_new {
        players.insert(name.clone(), 0);
    }

    let coins = *players.get(&name).unwrap();
    Json(json!({
        "success": true,
        "name": name,
        "coins": coins,
        "newPlayer": is_new,
    }))
}

/// GET /api/verify?name=xxx
/// 验证玩家是否存在并返回余额
async fn verify(State(players): State<Players>, Query(q): Query<NameQuery>) -> Json<Value> {
    match players.get(&q.name) {
        Some(coins) => Json(json!({ "success": true, "coins": *coins })),
        None => Json(json!({ "success": false, "msg": "玩家不存在" })),
    }
}

/// GET /api/players
/// 获取财富排行榜
async fn get_all_players(State(players): State<Players>) -> Json<Value> {
    let mut list: Vec<Value> = players
        .iter()
        .map(|e| json!({ "name": e.key(), "coins": *e.value() }))
        .collect();

    // 排序：金币从高到低
    list.sort_by(|a, b| {
        let a_val = a["coins"].as_i64().unwrap_or(0);
        let b_val = b["coins"].as_i64().unwrap_or(0);
        b_val.cmp(&a_val)
    });

    Json(json!({ "success": true, "players": list }))
}

/// GET /api/transfer?from=A&to=B&amount=100
/// 玩家间转账
async fn transfer(State(players): State<Players>, Query(q): Query<TransferQuery>) -> Json<Value> {
    let from = q.from.trim().to_string();
    let to = q.to.trim().to_string();

    if q.amount <= 0 {
        return Json(json!({ "success": false, "msg": "转账金额必须为正数" }));
    }
    if from == to {
        return Json(json!({ "success": false, "msg": "不能给自己转账" }));
    }

    // 检查转出者
    let from_coins = match players.get(&from) {
        Some(c) => *c,
        None => return Json(json!({ "success": false, "msg": "转出玩家不存在" })),
    };

    if from_coins < q.amount {
        return Json(json!({
            "success": false,
            "msg": format!("余额不足，当前余额：{}", format_coins(from_coins))
        }));
    }

    // 检查转入者
    if !players.contains_key(&to) {
        return Json(json!({ "success": false, "msg": "转入玩家不存在" }));
    }

    // 执行原子操作（简单逻辑直接用 get_mut）
    if let Some(mut f) = players.get_mut(&from) {
        *f -= q.amount;
    }
    if let Some(mut t) = players.get_mut(&to) {
        *t += q.amount;
    }

    Json(json!({
        "success": true,
        "msg": "转账成功",
        "fromCoins": *players.get(&from).unwrap(),
        "toCoins": *players.get(&to).unwrap(),
    }))
}

/// GET /api/paybank?name=xxx&amount=xxx
/// 还款给银行（扣除金币）
async fn pay_bank(State(players): State<Players>, Query(q): Query<PayBankQuery>) -> Json<Value> {
    let name = q.name.trim().to_string();
    if q.amount <= 0 {
        return Json(json!({ "success": false, "msg": "金额必须为正数" }));
    }

    match players.get_mut(&name) {
        None => Json(json!({ "success": false, "msg": "玩家不存在" })),
        Some(mut coins) => {
            if *coins < q.amount {
                return Json(json!({
                    "success": false,
                    "msg": format!("余额不足，当前余额：{}", format_coins(*coins))
                }));
            }
            *coins -= q.amount;
            Json(json!({ "success": true, "msg": "扣除成功", "coins": *coins }))
        }
    }
}

// =====================================================================
// ===== 管理员接口 (Admin Handlers) =====
// =====================================================================

/// GET /api/admin/verify?password=xxx
async fn admin_verify(Query(q): Query<AdminPasswordQuery>) -> Json<Value> {
    if q.password == ADMIN_PASSWORD {
        Json(json!({ "success": true }))
    } else {
        Json(json!({ "success": false, "msg": "密码错误" }))
    }
}

/// GET /api/admin/reset?password=xxx
/// 重置所有玩家金币为 0
async fn admin_reset(State(players): State<Players>, Query(q): Query<AdminPasswordQuery>) -> Json<Value> {
    if let Some(err) = check_admin(&q.password) {
        return Json(err);
    }

    // 关键点：DashMap 的 alter_all 闭包需要返回新值
    players.alter_all(|_, _| 0);

    Json(json!({ "success": true, "msg": "所有玩家金币已归零" }))
}

/// GET /api/admin/addcoins?password=xxx&name=xxx&amount=xxx
/// 管理员手动为某人加钱
async fn admin_add_coins(State(players): State<Players>, Query(q): Query<AdminAddQuery>) -> Json<Value> {
    if let Some(err) = check_admin(&q.password) {
        return Json(err);
    }

    let name = q.name.trim().to_string();
    if q.amount <= 0 {
        return Json(json!({ "success": false, "msg": "金额必须为正数" }));
    }

    match players.get_mut(&name) {
        None => Json(json!({ "success": false, "msg": "玩家不存在" })),
        Some(mut coins) => {
            *coins += q.amount;
            Json(json!({ "success": true, "msg": "增加成功", "coins": *coins }))
        }
    }
}

/// GET /api/admin/giveall?password=xxx&amount=xxx
/// 全体发放低保/奖励
async fn admin_give_all(State(players): State<Players>, Query(q): Query<AdminGiveAllQuery>) -> Json<Value> {
    if let Some(err) = check_admin(&q.password) {
        return Json(err);
    }
    if q.amount <= 0 {
        return Json(json!({ "success": false, "msg": "金额必须为正数" }));
    }

    // 关键点：DashMap 的 alter_all 闭包需要返回新值
    players.alter_all(|_, v| v + q.amount);

    let count = players.len();
    Json(json!({
        "success": true,
        "msg": format!("已向所有玩家发放 {}", format_coins(q.amount)),
        "count": count,
    }))
}

/// GET /api/admin/delete?password=xxx&names=A,B,C
/// 批量删除玩家
async fn admin_delete(State(players): State<Players>, Query(q): Query<AdminDeleteQuery>) -> Json<Value> {
    if let Some(err) = check_admin(&q.password) {
        return Json(err);
    }

    let deleted: Vec<String> = q.names
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|name| !name.is_empty())
        .filter(|name| players.remove(name).is_some())
        .collect();

    Json(json!({
        "success": true,
        "deleted": deleted,
        "msg": format!("已删除玩家: {}", deleted.join(", ")),
    }))
}

// =====================================================================
// ===== 主程序入口 =====
// =====================================================================

#[tokio::main]
async fn main() {
    // 初始化内存存储
    let players: Players = Arc::new(DashMap::new());

    // 跨域设置
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_origin(Any);

    // 路由映射
    let app = Router::new()
        // --- 玩家相关 ---
        .route("/api/login",         get(login))
        .route("/api/verify",        get(verify))
        .route("/api/players",       get(get_all_players))
        .route("/api/transfer",      get(transfer))
        .route("/api/paybank",       get(pay_bank))
        // --- 管理员相关 ---
        .route("/api/admin/verify",  get(admin_verify))
        .route("/api/admin/reset",   get(admin_reset))
        .route("/api/admin/addcoins",get(admin_add_coins))
        .route("/api/admin/giveall", get(admin_give_all))
        .route("/api/admin/delete",  get(admin_delete))
        // 中间件与状态
        .layer(cors)
        .with_state(players);

    // 绑定地址
    let addr = "0.0.0.0:8860";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    println!(r#"
    🎩 Monopoly Bank Server Started!
    ----------------------------------
    Local:  http://127.0.0.1:8080
    Admin:  Password is "{}"
    ----------------------------------
    "#, ADMIN_PASSWORD);

    axum::serve(listener, app).await.unwrap();
}