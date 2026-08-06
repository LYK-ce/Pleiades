//Presented by KeJi
//Date ： 2026-05-17
//Modified Date ： 2026-08-06

//! Pleiades 入口点（PC 纯推理节点，Robot 已移除，Task 9_2）

use pleiades::bootstrap::core_bootstrap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let boot = core_bootstrap().await?;
    boot.run().await;
    Ok(())
}
