use crate::{
    error::AppError,
    plugins::node_detect::{self, NodeToolchainStatus},
};

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatusRequest {
    pub custom_node_path: Option<String>,
}

#[tauri::command]
pub async fn plugins_node_status(
    request: NodeStatusRequest,
) -> Result<NodeToolchainStatus, AppError> {
    node_detect::detect(request.custom_node_path).await
}
