#[allow(clippy::wildcard_imports)]
use super::*;

/// `ModalFormCancelReason::UserBusy`: the client refused to show the form because another screen is open.
const CANCEL_REASON_USER_BUSY: u8 = 1;

impl BedrockClient {
    pub fn handle_modal_form_response(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: pumpkin_protocol::bedrock::server::modal_form_response::SModalFormResponse<'_>,
    ) {
        // Plugins see only the response text, and a busy client needs the form shown again, unlike
        // a closed one. No form type answers with a JSON string, so this cannot be taken for an answer.
        let response = if packet.form_cancel_reason == Some(CANCEL_REASON_USER_BUSY) {
            Some("\"busy\"".to_owned())
        } else {
            packet
                .json_response
                .filter(|data| data != "null")
                .map(std::borrow::Cow::into_owned)
        };
        let mut event =
            crate::plugin::api::events::player::bedrock_form_response::BedrockFormResponseEvent::new(
                player.clone(),
                packet.form_id.0,
                response,
            );
        server.plugin_manager.fire_blocking(server, &mut event);
    }
}
