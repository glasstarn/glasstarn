//! Following one item type of a connected log in the browser.
//!
//! The connected (OCEL) source moves to its own slot and the followed case log
//! takes the ordinary upload slot, so every case view (map, variants, timing,
//! cases, filters, comparison) works on it unchanged. Connected-only views keep
//! reading the source through [`Server::ocel_store`]. The project itself stays
//! connected: saving stores the source, never the followed case log.
use super::*;
use serde_json::{Value, json};

pub(super) struct FollowContext {
    pub(super) object_type: String,
    pub(super) base_filter: pm_core::OcelFilter,
    pub(super) case_filter: pm_core::Filter,
    pub(super) journey: Option<pm_query::Journey>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectedScopeCommand {
    ocel_upload_id: String,
    object_type: String,
    case_view: ComparisonOptionsRequest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowCommand {
    #[serde(default)]
    preserve_scope: bool,
    /// The connected source, or the followed log already in place.
    upload_id: String,
    object_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JourneyCommand {
    upload_id: String,
    object_type: String,
}

impl Server {
    pub(super) fn connected_scope_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let command: ConnectedScopeCommand = match parse_json_request(stream, &request) {
            Ok(command) => command,
            Err(error) => return write_request_error(stream, error, "connected scope"),
        };
        let result = (|| -> Result<Value, String> {
            let mut context = self
                .follow_context
                .lock()
                .map_err(|_| "Follow context unavailable")?;
            let context = context
                .as_mut()
                .filter(|context| context.object_type == command.object_type)
                .ok_or("Follow this item type before requesting its scope")?;
            let source = self
                .ocel_store()
                .lock()
                .map_err(|_| "Connected source unavailable")?;
            let source = source
                .as_ref()
                .filter(|source| source.id == command.ocel_upload_id)
                .ok_or("Connected source changed; open the project again")?;
            if context.journey.is_none() {
                context.journey = Some(
                    pm_query::ocel_journey(
                        &source.path,
                        &context.base_filter,
                        &command.object_type,
                    )
                    .map_err(|error| error.to_string())?,
                );
            }
            let plan = context.journey.as_ref().unwrap();
            let mut filter = context.base_filter.clone();
            context.case_filter = command.case_view.filter.clone();
            let narrowed = !command.case_view.filter.is_empty();
            if narrowed {
                let log = self
                    .load_comparison_log(
                        &command.case_view.upload_id,
                        command.case_view.mapping,
                        &command.case_view.policy,
                        command.case_view.rules,
                        command.case_view.filter,
                    )
                    .map_err(|_| "Could not resolve the filtered items")?;
                let roots = log
                    .traces
                    .iter()
                    .map(|trace| log.case_label(trace.case).to_owned())
                    .collect();
                filter.cohort = Some(pm_core::OcelCohort {
                    parent: filter.cohort.map(Box::new),
                    root_type: command.object_type.clone(),
                    roots,
                    paths: plan
                        .handoffs
                        .iter()
                        .map(|handoff| handoff.scope_path.clone())
                        .collect(),
                });
            }
            let journey = if narrowed {
                pm_query::measure_journey_plan(&source.path, &filter, plan)
                    .map_err(|error| error.to_string())?
            } else {
                plan.clone()
            };
            let mut analysis =
                pm_query::analyze_ocel(&source.path, &filter).map_err(|error| error.to_string())?;
            analysis.source.clone_from(&source.file_name);
            if let Some(log) = self
                .live_workspace
                .lock()
                .map_err(|_| "Live scope unavailable")?
                .ocel_log
                .as_mut()
            {
                log.filter = filter;
            }
            Ok(
                json!({"journey":journey,"analysis":analysis,"root_type":command.object_type,"narrowed":narrowed}),
            )
        })();
        match result {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err(message) => write_json(
                stream,
                422,
                "Unprocessable Content",
                &ErrorResponse::new(&message),
            ),
        }
    }
    pub(super) fn journey_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        let result = (|| -> Result<Value, (u16, String)> {
            let body = request
                .json_body(stream)
                .map_err(|(status, _, message)| (status, message.to_owned()))?;
            let command: JourneyCommand = serde_json::from_slice(&body)
                .map_err(|_| (422, "send the connected source and an item type".to_owned()))?;
            let filter = self
                .live_workspace
                .lock()
                .map_err(|_| (422, "the live workspace is unavailable".to_owned()))?
                .ocel_log
                .as_ref()
                .map(|log| log.filter.clone())
                .unwrap_or_default();
            let store = self
                .ocel_store()
                .lock()
                .map_err(|_| (422, "the upload store is unavailable".to_owned()))?;
            let source = store
                .as_ref()
                .filter(|source| source.id == command.upload_id)
                .ok_or((404, "the connected file is no longer available".to_owned()))?;
            let journey = pm_query::ocel_journey(&source.path, &filter, &command.object_type)
                .map_err(|error| (422, error.to_string()))?;
            serde_json::to_value(journey).map_err(|error| (422, error.to_string()))
        })();
        match result {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err((status, message)) => {
                let reason = if status == 404 {
                    "Not Found"
                } else {
                    "Unprocessable Content"
                };
                write_json(stream, status, reason, &ErrorResponse::new(&message))
            }
        }
    }

    /// Where the connected source lives: its own slot while an item type is
    /// followed, otherwise the ordinary upload slot.
    pub(super) fn ocel_store(&self) -> &Mutex<Option<TemporaryUpload>> {
        if self.ocel_upload.lock().is_ok_and(|stored| stored.is_some()) {
            &self.ocel_upload
        } else {
            &self.upload
        }
    }

    /// Drops any followed view; called when a new file replaces the data.
    pub(super) fn stop_following(&self) {
        if let Ok(mut context) = self.follow_context.lock() {
            *context = None;
        }
        if let Ok(mut stored) = self.ocel_upload.lock() {
            *stored = None;
        }
    }

    pub(super) fn following(&self) -> bool {
        self.ocel_upload.lock().is_ok_and(|stored| stored.is_some())
    }

    pub(super) fn follow_request(
        &self,
        stream: &mut TcpStream,
        request: Request,
    ) -> Result<(), ServerError> {
        match self.follow(stream, request) {
            Ok(value) => write_json(stream, 200, "OK", &value),
            Err((status, message)) => {
                let reason = if status == 404 {
                    "Not Found"
                } else {
                    "Unprocessable Content"
                };
                write_json(stream, status, reason, &ErrorResponse::new(&message))
            }
        }
    }

    fn follow(&self, stream: &mut TcpStream, request: Request) -> Result<Value, (u16, String)> {
        let body = request
            .json_body(stream)
            .map_err(|(status, _, message)| (status, message.to_owned()))?;
        let command: FollowCommand = serde_json::from_slice(&body)
            .map_err(|_| (422, "send the connected source and an item type".to_owned()))?;
        let mut filter = self
            .live_workspace
            .lock()
            .map_err(|_| (422, "the live workspace is unavailable".to_owned()))?
            .ocel_log
            .as_ref()
            .map(|log| log.filter.clone())
            .ok_or((
                404,
                "open a connected log before following an item type".to_owned(),
            ))?;
        if !command.preserve_scope
            && let Some(context) = self
                .follow_context
                .lock()
                .map_err(|_| (422, "Follow context unavailable".to_owned()))?
                .as_ref()
        {
            filter = context.base_filter.clone();
        }
        let mut followed = TemporaryUpload::create(".csv")
            .map_err(|_| (422, "cannot stage the followed log".to_owned()))?;
        // Identify the source without holding two locks on one store: before
        // the first follow the source *is* the ordinary upload.
        let current_id = self
            .upload
            .lock()
            .map_err(|_| (422, "the upload store is unavailable".to_owned()))?
            .as_ref()
            .map(|upload| upload.id.clone());
        let (summary, source_id, source_name) = {
            let store = self
                .ocel_store()
                .lock()
                .map_err(|_| (422, "the upload store is unavailable".to_owned()))?;
            let source = store
                .as_ref()
                .filter(|source| {
                    source.id == command.upload_id
                        || current_id.as_deref() == Some(command.upload_id.as_str())
                })
                .ok_or((
                    404,
                    "the connected file is no longer available; open the project again".to_owned(),
                ))?;
            let file = followed.file.as_mut().expect("a new upload is open");
            let summary =
                pm_query::follow_ocel(&source.path, &filter, &command.object_type, &mut *file)
                    .map_err(|error| (422, error.to_string()))?;
            (summary, source.id.clone(), source.file_name.clone())
        };
        followed.file = None;
        *self
            .follow_context
            .lock()
            .map_err(|_| (422, "Follow context unavailable".to_owned()))? = Some(FollowContext {
            case_filter: Default::default(),
            object_type: command.object_type.clone(),
            base_filter: filter,
            journey: None,
        });
        followed.file_name = format!("{} followed in {source_name}", command.object_type);
        followed.bytes = fs::metadata(&followed.path).map(|m| m.len()).unwrap_or(0);
        let followed_id = followed.id.clone();
        // The first follow moves the connected source to its own slot.
        {
            let mut current = self
                .upload
                .lock()
                .map_err(|_| (422, "the upload store is unavailable".to_owned()))?;
            let mut source = self
                .ocel_upload
                .lock()
                .map_err(|_| (422, "the upload store is unavailable".to_owned()))?;
            if source.is_none() {
                *source = current.take();
            }
            *current = Some(followed);
        }
        Ok(
            json!({"upload_id":followed_id,"ocel_upload_id":source_id,"follow":summary,
            "mapping":summary.mapping(),"file_name":format!("{} followed", command.object_type)}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};

    fn call(server: &Server, path: &str, body: &[u8], file: Option<&str>) -> (u16, Value) {
        let mut stream = TcpStream::connect(server.address).unwrap();
        let name = file
            .map(|n| format!("X-File-Name: {n}\r\n"))
            .unwrap_or_default();
        write!(stream, "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n{name}Content-Length: {}\r\n\r\n", server.address, server.token(), body.len()).unwrap();
        stream.write_all(body).unwrap();
        let (incoming, _) = server.listener.accept().unwrap();
        server.handle(incoming).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        (
            status,
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
        )
    }

    #[test]
    fn following_an_item_type_feeds_the_case_views_and_keeps_the_project_connected() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::bind(0).unwrap();
        *server.workspace_folder.lock().unwrap() = Some(
            WorkspaceFolder::create_named(
                &dir.path().join("p"),
                "Orders",
                InvestigationPurpose::Improvement,
            )
            .unwrap(),
        );
        let fixture = include_bytes!("../../../testdata/ocel/order-management.jsonocel");
        let (status, opened) = call(&server, "/api/ocel", fixture, Some("orders.jsonocel"));
        assert_eq!(status, 200, "{opened}");
        let source = opened["upload_id"].as_str().unwrap().to_owned();

        let request = json!({"upload_id":source,"object_type":"item"}).to_string();
        let (status, followed) = call(&server, "/api/ocel/follow", request.as_bytes(), None);
        assert_eq!(status, 200, "{followed}");
        assert_eq!(followed["follow"]["cases"], 2);
        assert_eq!(followed["ocel_upload_id"], source.as_str());

        // The followed log goes through the ordinary case analysis.
        let analyze =
            json!({"upload_id":followed["upload_id"],"mapping":followed["mapping"]}).to_string();
        let (status, analysis) = call(&server, "/api/analyze", analyze.as_bytes(), None);
        assert_eq!(status, 200, "{analysis}");
        assert_eq!(analysis["analysis"]["summary"]["cases"], 2);

        let scope = json!({"ocel_upload_id":source,"object_type":"item","case_view":{"upload_id":followed["upload_id"],"mapping":followed["mapping"],"filter":{"without_activities":["place order"]}}}).to_string();
        let (status, empty) = call(&server, "/api/ocel/scope", scope.as_bytes(), None);
        assert_eq!(status, 200, "{empty}");
        assert_eq!(empty["analysis"]["source"], "orders.jsonocel");
        assert_eq!(empty["analysis"]["scope"]["events_shown"], 0, "{empty}");
        assert_eq!(empty["journey"]["stages"].as_array().unwrap().len(), 0);
        assert!(
            server
                .live_workspace
                .lock()
                .unwrap()
                .ocel_log
                .as_ref()
                .unwrap()
                .filter
                .cohort
                .as_ref()
                .unwrap()
                .roots
                .is_empty()
        );
        server.save_workspace().unwrap();
        {
            let folder = server.workspace_folder.lock().unwrap();
            let folder = folder.as_ref().unwrap();
            let state = folder.state().unwrap();
            let saved = state.followed_view.as_ref().unwrap();
            assert_eq!(saved.object_type, "item");
            assert!(saved.base_filter.cohort.is_none());
            assert!(!saved.filter.is_empty());
            let restored = Server::bind(0).unwrap();
            restored.restore_folder(folder).unwrap();
            let upload = restored.upload.lock().unwrap().as_ref().unwrap().id.clone();
            let request = json!({"upload_id":upload,"object_type":"item"}).to_string();
            let (status, again) = call(&restored, "/api/ocel/follow", request.as_bytes(), None);
            assert_eq!(status, 200, "{again}");
            assert_eq!(
                again["follow"]["cases"], 2,
                "an empty saved cohort must remain recoverable from its original scope"
            );
        }
        let scope = json!({"ocel_upload_id":source,"object_type":"item","case_view":{"upload_id":followed["upload_id"],"mapping":followed["mapping"]}}).to_string();
        let (status, restored) = call(&server, "/api/ocel/scope", scope.as_bytes(), None);
        assert_eq!(status, 200, "{restored}");
        assert!(!restored["narrowed"].as_bool().unwrap());

        // Connected views still read the connected source.
        let ocel = json!({"upload_id":source}).to_string();
        let (status, connected) = call(&server, "/api/ocel/analyze", ocel.as_bytes(), None);
        assert_eq!(status, 200, "{connected}");
        assert_eq!(connected["analysis"]["scope"]["events_total"], 3);

        // Switching type works from either identifier, and saving keeps the source.
        let request = json!({"upload_id":followed["upload_id"],"object_type":"order"}).to_string();
        let (status, orders) = call(&server, "/api/ocel/follow", request.as_bytes(), None);
        assert_eq!(status, 200, "{orders}");
        assert_eq!(orders["follow"]["cases"], 1);
        server.save_workspace().unwrap();
        let folder = server.workspace_folder.lock().unwrap();
        let state = folder.as_ref().unwrap().state().unwrap();
        assert!(state.source.unwrap().name.ends_with(".jsonocel"));
        assert!(state.case_project.is_none());
        assert!(state.ocel_filter.is_some());
    }
}
