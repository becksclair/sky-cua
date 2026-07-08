use super::*;

impl ServiceDaemon {
    pub(super) async fn handle_browser_request(&self, request: BrowserRequest) -> ServiceResponse {
        // Any browser request marks the session active so the daemon's idle
        // exit cannot kill the heartbeat keepalive (and with it every tab's
        // debugger attachment) between an agent's browser actions.
        crate::browser::mark_bridge_activity();
        match request {
            BrowserRequest::ListTabs { target } => {
                debug!(?target, "handling browser_list_tabs request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ListTabs {
                        response: crate::browser::list_tabs(target).await,
                    },
                }
            }
            BrowserRequest::Open { target, url } => {
                debug!(?target, ?url, "handling browser_open request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Open {
                        response: crate::browser::open_tab(target, url).await,
                    },
                }
            }
            BrowserRequest::ClaimTab { target, tab_id } => {
                debug!(?target, ?tab_id, "handling browser_claim_tab request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ClaimTab {
                        response: crate::browser::claim_tab(target, tab_id).await,
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
                        response: crate::browser::move_mouse(
                            target,
                            tab_id,
                            x,
                            y,
                            wait_for_arrival,
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
                        response: crate::browser::navigate(target, tab_id, url).await,
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
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot text_limit must be at most {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT}"
                        ),
                    };
                }
                if element_limit.is_some_and(|value| value > BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT) {
                    return ServiceResponse::Error {
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot element_limit must be at most {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}"
                        ),
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
                        response: crate::browser::snapshot(
                            target,
                            tab_id,
                            text_limit,
                            element_offset,
                            element_limit,
                            element_query,
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
                        response: crate::browser::screenshot(target, tab_id, include_image_data)
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
                        response: crate::browser::click(target, tab_id, x, y).await,
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
                        response: crate::browser::type_text(target, tab_id, text).await,
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
                        response: crate::browser::press_key(target, tab_id, key).await,
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
                        response: crate::browser::scroll(target, tab_id, delta_x, delta_y, x, y)
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
                        response: crate::browser::eval_with_policy(
                            target,
                            tab_id,
                            expression,
                            self.browser_eval_enabled,
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
        let integration = {
            let Ok(_desktop_lane) = self.desktop_lane.try_lock() else {
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

        ServiceResponse::Browser {
            response: BrowserResponse::Status {
                report: crate::browser::browser_status_from_doctor(integration).await,
            },
        }
    }
}
