//! 服务层（design §1.2）：每个服务拼请求、解析响应。输入是普通结构体，不读全局配置；
//! 只依赖平台层（系统代理）。加服务 = 加一个文件 + `logic::translate` 的 match 加一行。

pub mod ai;
pub mod google;
pub mod http;
pub mod umi;
