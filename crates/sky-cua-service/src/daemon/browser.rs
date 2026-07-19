use super::*;

impl ServiceDaemon {
    pub(super) async fn handle_browser_request(
        &self,
        request: BrowserRequest,
        identity: Option<BrowserSessionIdentity>,
        context: Option<sky_cua_platform::model::BrowserRequestContext>,
    ) -> ServiceResponse {
        // Any browser request marks the session active so the daemon's idle
        // exit cannot kill the heartbeat keepalive (and with it every tab's
        // debugger attachment) between an agent's browser actions.
        crate::browser::mark_bridge_activity();
        match &self.browser_control_mode {
            Err(diagnostic) => return error_response(&diagnostic.code, &diagnostic.message),
            Ok(mode) if mode.uses_persistent_actor() => {
                let Some(runtime) = &self.browser_control_runtime else {
                    return error_response(
                        "BrowserControlUnavailable",
                        "persistent browser control runtime did not initialize",
                    );
                };
                let Some(context) = context else {
                    return error_response(
                        "BrowserRequestContextRequired",
                        "hybrid/strict browser requests require BrowserRequestContext",
                    );
                };
                runtime.observe_mcp_client(&context.provenance);
                if !matches!(request, BrowserRequest::Status) {
                    return match runtime.high_level(request, context).await {
                        Ok(response) => ServiceResponse::Browser { response },
                        Err(diagnostic) => error_response(&diagnostic.code, &diagnostic.message),
                    };
                }
            }
            Ok(_) => {}
        }
        match request {
            BrowserRequest::ListTabs { target } => {
                debug!(?target, "handling browser_list_tabs request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ListTabs {
                        response: crate::browser::list_tabs_with_identity(target, identity).await,
                    },
                }
            }
            BrowserRequest::Open { target, url } => {
                debug!(?target, ?url, "handling browser_open request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Open {
                        response: crate::browser::open_tab_with_identity(target, url, identity)
                            .await,
                    },
                }
            }
            BrowserRequest::ClaimTab { target, tab_id } => {
                debug!(?target, ?tab_id, "handling browser_claim_tab request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ClaimTab {
                        response: crate::browser::claim_tab_with_identity(target, tab_id, identity)
                            .await,
                    },
                }
            }
            BrowserRequest::MoveMouse {
                target,
                tab_id,
                x,
                y,
                wait_for_arrival,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    x,
                    y,
                    wait_for_arrival,
                    "handling browser_move_mouse request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::MoveMouse {
                        response: crate::browser::move_mouse_with_identity(
                            target,
                            tab_id,
                            x,
                            y,
                            wait_for_arrival,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Navigate {
                target,
                tab_id,
                url,
            } => {
                debug!(?target, ?tab_id, ?url, "handling browser_navigate request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Navigate {
                        response: crate::browser::navigate_with_identity(
                            target, tab_id, url, identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Snapshot {
                target,
                tab_id,
                text_limit,
                element_offset,
                element_limit,
                element_query,
            } => {
                if text_limit.is_some_and(|value| value > BROWSER_SNAPSHOT_MAX_TEXT_LIMIT) {
                    return ServiceResponse::Error {
                        ok: false,
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot text_limit must be at most {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT}"
                        ),
                        session_id: None,
                        turn_id: None,
                        retry: None,
                    };
                }
                if element_limit.is_some_and(|value| value > BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT) {
                    return ServiceResponse::Error {
                        ok: false,
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot element_limit must be at most {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}"
                        ),
                        session_id: None,
                        turn_id: None,
                        retry: None,
                    };
                }
                debug!(
                    ?target,
                    ?tab_id,
                    ?text_limit,
                    ?element_offset,
                    ?element_limit,
                    ?element_query,
                    "handling browser_snapshot request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Snapshot {
                        response: crate::browser::snapshot_with_identity(
                            target,
                            tab_id,
                            text_limit,
                            element_offset,
                            element_limit,
                            element_query,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Screenshot {
                target,
                tab_id,
                include_image_data,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    ?include_image_data,
                    "handling browser_screenshot request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Screenshot {
                        response: crate::browser::screenshot_with_identity(
                            target,
                            tab_id,
                            include_image_data,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Click {
                target,
                tab_id,
                x,
                y,
            } => {
                debug!(?target, ?tab_id, x, y, "handling browser_click request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Click {
                        response: crate::browser::click_with_identity(
                            target, tab_id, x, y, identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::ClickElement {
                target,
                tab_id,
                element_ref,
            } => {
                debug!(?target, ?tab_id, "handling browser_click element request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Click {
                        response: crate::browser::click_element_with_identity(
                            target,
                            tab_id,
                            element_ref,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::TypeText {
                target,
                tab_id,
                text,
            } => {
                debug!(?target, ?tab_id, "handling browser_type_text request");
                ServiceResponse::Browser {
                    response: BrowserResponse::TypeText {
                        response: crate::browser::type_text_with_identity(
                            target, tab_id, text, identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::TypeTextElement {
                target,
                tab_id,
                element_ref,
                text,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    "handling browser_type_text element request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::TypeText {
                        response: crate::browser::type_text_element_with_identity(
                            target,
                            tab_id,
                            element_ref,
                            text,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::PressKey {
                target,
                tab_id,
                key,
            } => {
                debug!(?target, ?tab_id, ?key, "handling browser_press_key request");
                ServiceResponse::Browser {
                    response: BrowserResponse::PressKey {
                        response: crate::browser::press_key_with_identity(
                            target, tab_id, key, identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Scroll {
                target,
                tab_id,
                delta_x,
                delta_y,
                x,
                y,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    delta_x,
                    delta_y,
                    x,
                    y,
                    "handling browser_scroll request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Scroll {
                        response: crate::browser::scroll_with_identity(
                            target, tab_id, delta_x, delta_y, x, y, identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Eval {
                target,
                tab_id,
                expression,
            } => {
                debug!(?target, ?tab_id, "handling browser_eval request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Eval {
                        response: crate::browser::eval_with_policy_and_identity(
                            target,
                            tab_id,
                            expression,
                            self.browser_eval_enabled,
                            identity,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Status => self.handle_browser_status_request().await,
        }
    }

    async fn handle_browser_status_request(&self) -> ServiceResponse {
        debug!("handling browser_status request");
        let persistent_runtime = self.browser_control_runtime.as_ref();
        let integration = {
            let Ok(_desktop_lane) = self.desktop_lane.try_lock() else {
                if let Some(runtime) = persistent_runtime {
                    let mut report = runtime.status_report(None, true).await;
                    self.append_browser_control_startup_diagnostics(&mut report);
                    return ServiceResponse::Browser {
                        response: BrowserResponse::Status { report },
                    };
                }
                return ServiceResponse::Browser {
                    response: BrowserResponse::Status {
                        report: crate::browser::browser_status_from_deferred_doctor().await,
                    },
                };
            };
            match self.backend.doctor().await {
                Ok(report) => report.browser_integration,
                Err(error) => return error_response(error.code, error.message),
            }
        };

        if let Some(runtime) = persistent_runtime {
            let mut report = runtime.status_report(integration, false).await;
            self.append_browser_control_startup_diagnostics(&mut report);
            return ServiceResponse::Browser {
                response: BrowserResponse::Status { report },
            };
        }

        ServiceResponse::Browser {
            response: BrowserResponse::Status {
                report: crate::browser::browser_status_from_doctor(integration).await,
            },
        }
    }
}
