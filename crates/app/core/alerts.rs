use crate::{
	heuristics::{
		ALERT_METRICS_HEARTBEAT_DURATION_PRODUCTION, ALERT_METRICS_HEARTBEAT_DURATION_TESTING,
		ALERT_METRICS_MINIMUM_PRODUCTION_METRICS_DEBUG_THRESHOLD,
		ALERT_METRICS_MINIMUM_PRODUCTION_METRICS_THRESHOLD,
	},
	model::get_model_bytes,
	production_metrics::{ProductionMetrics, ProductionPredictionMetricsOutput},
	App, AppState,
};
use anyhow::{anyhow, Result};
use futures::{select, FutureExt};
//use lettre::AsyncTransport;
use serde::{Deserialize, Serialize};
use sqlx::prelude::*;
use std::{borrow::BorrowMut, fmt, io, str::FromStr, sync::Arc};
use tangram_id::Id;
use tokio::sync::Notify;

// Task

// Types

/// Alert cadence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum AlertCadence {
	#[serde(rename = "testing")]
	Testing,
	#[serde(rename = "hourly")]
	Hourly,
	#[serde(rename = "daily")]
	Daily,
	#[serde(rename = "weekly")]
	Weekly,
	#[serde(rename = "monthly")]
	Monthly,
}

// store timestamp, truncate to significant portion.

impl AlertCadence {
	pub fn duration(&self) -> tokio::time::Duration {
		// This is really not a duration, it's looking for the time until the next interval?
		// think about this more
		todo!()
	}
}

impl Default for AlertCadence {
	fn default() -> Self {
		AlertCadence::Hourly
	}
}

impl fmt::Display for AlertCadence {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let s = match self {
			AlertCadence::Testing => "Testing",
			AlertCadence::Hourly => "Hourly",
			AlertCadence::Daily => "Daily",
			AlertCadence::Weekly => "Weekly",
			AlertCadence::Monthly => "Monthly",
		};
		write!(f, "{}", s)
	}
}

impl FromStr for AlertCadence {
	type Err = io::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"daily" => Ok(AlertCadence::Daily),
			"hourly" => Ok(AlertCadence::Hourly),
			"monthly" => Ok(AlertCadence::Monthly),
			"testing" => Ok(AlertCadence::Testing),
			"weekly" => Ok(AlertCadence::Weekly),
			_ => Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				format!("Unsupported cadence {}", s),
			)),
		}
	}
}

/// The various ways to receive alerts
// FIXME - using tag = type and renaming here causes sqlx to panic!!
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum AlertMethod {
	/// Send an email to the stored address
	Email(String),
	/// Dump the alert to STDOUT - mostly useful for testing
	Stdout,
	/// POST the alert to the given URL as a webhook
	Webhook(String),
}

impl AlertMethod {
	pub async fn send_alert(
		&self,
		exceeded_thresholds: &[&AlertResult],
		_context: &AppState,
	) -> Result<()> {
		match self {
			AlertMethod::Email(_address) => {
				// TODO re-enable this code!
				/*
				let email = lettre::Message::builder()
					.from("Tangram <noreply@tangram.dev>".parse()?)
					.to(address.parse()?)
					.subject("Tangram Metrics Alert")
					.body(format!(
						"Exceeded alert thresholds: {:?}",
						exceeded_thresholds
					))?;
				if let Some(smtp_transport) = &context.smtp_transport {
					smtp_transport.send(email).await?;
				} else {
					return Err(anyhow!("No SMTP Transport in context"));
				}
				*/
			}
			AlertMethod::Stdout => println!("exceeded thresholds: {:?}", exceeded_thresholds),
			AlertMethod::Webhook(_url) => {
				// Spawn a thread
				// Attempt the POST, record status in DB.
				// If status has failed, attempt again until it succeeds.
			}
		}
		Ok(())
	}
}

/// Statistics that can generate alerts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum AlertMetric {
	#[serde(rename = "accuracy")]
	Accuracy,
	#[serde(rename = "mean_squared_error")]
	MeanSquaredError,
	#[serde(rename = "root_mean_squared_error")]
	RootMeanSquaredError,
}

impl AlertMetric {
	pub fn short_name(&self) -> String {
		match self {
			AlertMetric::Accuracy => "accuracy".to_owned(),
			AlertMetric::MeanSquaredError => "mse".to_owned(),
			AlertMetric::RootMeanSquaredError => "rmse".to_owned(),
		}
	}

	/// Check if the given AlertModelType is applicable to this AlertMetric
	pub fn validate(&self, model_type: AlertModelType) -> bool {
		match self {
			AlertMetric::Accuracy => matches!(model_type, AlertModelType::Classifier),
			AlertMetric::MeanSquaredError | &AlertMetric::RootMeanSquaredError => {
				matches!(model_type, AlertModelType::Regressor)
			}
		}
	}
}

impl fmt::Display for AlertMetric {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let s = match self {
			AlertMetric::Accuracy => "Accuracy",
			AlertMetric::MeanSquaredError => "Mean Squared Error",
			AlertMetric::RootMeanSquaredError => "Root Mean Squared Error",
		};
		write!(f, "{}", s)
	}
}

impl FromStr for AlertMetric {
	type Err = io::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"accuracy" => Ok(AlertMetric::Accuracy),
			"mse" | "mean_squared_error" => Ok(AlertMetric::MeanSquaredError),
			"rmse" | "root_mean_squared_error" => Ok(AlertMetric::RootMeanSquaredError),
			_ => Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"Unsupported alert metric",
			)),
		}
	}
}

/// For filtering valid metric options
#[derive(Debug, Clone, Copy)]
pub enum AlertModelType {
	Classifier,
	Regressor,
}

impl From<tangram_model::ModelInnerReader<'_>> for AlertModelType {
	fn from(mir: tangram_model::ModelInnerReader) -> Self {
		use tangram_model::ModelInnerReader::*;
		match mir {
			BinaryClassifier(_) | MulticlassClassifier(_) => AlertModelType::Classifier,
			Regressor(_) => AlertModelType::Regressor,
		}
	}
}

/// Alerts can either be set as absolute values or percentage deviations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum MonitorThresholdMode {
	#[serde(rename = "absolute")]
	Absolute,
	#[serde(rename = "percentage")]
	Percentage,
}

impl fmt::Display for MonitorThresholdMode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let s = match self {
			MonitorThresholdMode::Absolute => "absolute",
			MonitorThresholdMode::Percentage => "percentage",
		};
		write!(f, "{}", s)
	}
}

impl FromStr for MonitorThresholdMode {
	type Err = io::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"absolute" => Ok(MonitorThresholdMode::Absolute),
			"percentage" => Ok(MonitorThresholdMode::Percentage),
			_ => Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"unsupported threshold mode",
			)),
		}
	}
}

/// Single alert threshold
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct MonitorThreshold {
	pub metric: AlertMetric,
	pub mode: MonitorThresholdMode,
	pub variance_lower: Option<f32>,
	pub variance_upper: Option<f32>,
}

impl Default for MonitorThreshold {
	fn default() -> Self {
		MonitorThreshold {
			metric: AlertMetric::Accuracy,
			mode: MonitorThresholdMode::Absolute,
			variance_lower: Some(0.1),
			variance_upper: Some(0.1),
		}
	}
}

pub fn extract_threshold_bounds(
	threshold_bounds: (String, String),
) -> Result<(Option<f32>, Option<f32>)> {
	let lower = if !threshold_bounds.0.is_empty() {
		Some(threshold_bounds.0.parse()?)
	} else {
		None
	};
	let upper = if !threshold_bounds.1.is_empty() {
		Some(threshold_bounds.1.parse()?)
	} else {
		None
	};
	Ok((lower, upper))
}

pub fn validate_threshold_bounds(lower: String, upper: String) -> Option<(String, String)> {
	let at_least_one = (!lower.is_empty() && !upper.is_empty())
		|| (lower.is_empty() && !upper.is_empty())
		|| (!lower.is_empty() && upper.is_empty());
	if at_least_one {
		Some((lower, upper))
	} else {
		None
	}
}

/// A result from checking a metric
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct AlertResult {
	pub metric: AlertMetric,
	pub observed_value: f32,
	pub observed_variance: f32,
}

impl AlertResult {
	/// Should this result send an alert?
	pub fn exceeds_threshold(&self, tolerance: f32) -> bool {
		self.observed_variance.abs() > tolerance
	}
}

/// Thresholds for generating an Alert
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Monitor {
	pub cadence: AlertCadence,
	pub id: Id,
	pub methods: Vec<AlertMethod>,
	pub model_id: Id,
	pub threshold: MonitorThreshold,
	pub title: String,
}

impl Monitor {
	pub fn get_thresholds(&self) -> (Option<f32>, Option<f32>) {
		(self.threshold.variance_upper, self.threshold.variance_lower)
	}
	/// Check if the given timestamp is more than one cadence interval behind the current time
	// FIXME - this is not correct.  Check when the last one was, when the next one should be, etc.
	// For now, just returns true all the time.
	pub fn is_overdue(&self) -> bool {
		//let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
		//let offset = now - timestamp;
		//offset > self.cadence.duration().as_secs()
		true
	}

	pub fn default_title(&self) -> String {
		format!("{} {}", self.cadence, self.threshold.metric)
	}
}

/// Manager for all enabled alerts
#[derive(Debug, Default)]
pub struct Monitors(Vec<Monitor>);

impl From<Vec<Monitor>> for Monitors {
	fn from(v: Vec<Monitor>) -> Self {
		Self(v)
	}
}

impl Monitors {
	// Retrieve all currently enabled cadences
	pub fn get_cadences(&self) -> Vec<AlertCadence> {
		self.0
			.iter()
			.map(|ah| ah.cadence)
			.collect::<Vec<AlertCadence>>()
	}
	pub fn alerts(&self) -> &[Monitor] {
		&self.0
	}

	/// Retrieve the heuristics for the given cadence, if present
	pub fn cadence(&self, cadence: AlertCadence) -> Option<&Monitor> {
		for heuristics in &self.0 {
			if heuristics.cadence == cadence {
				return Some(heuristics);
			}
		}
		None
	}
}

/// Collection for the alert results from a single run
#[derive(Debug, Deserialize, Serialize)]
pub struct AlertData {
	pub id: Id,
	pub monitor: Monitor,
	pub results: Vec<AlertResult>,
	pub timestamp: i64,
}

// Database interaction

pub async fn get_monitor(
	db: &mut sqlx::Transaction<'_, sqlx::Any>,
	monitor_id: Id,
) -> Result<Monitor> {
	let result = sqlx::query(
		"
			select
				data
			from
				monitors
			where
				id = $1
		",
	)
	.bind(monitor_id.to_string())
	.fetch_one(db)
	.await?;
	let monitor: String = result.get(0);
	let monitor: Monitor = serde_json::from_str(&monitor)?;
	Ok(monitor)
}

pub async fn get_overdue_monitors(app: &AppState) -> Result<Monitors> {
	let mut db = app.database_pool.begin().await?;
	let rows = sqlx::query(
		"
			select
				data
			from
				monitors
		",
	)
	.fetch_all(&mut db)
	.await?;
	db.commit().await?;
	let monitors: Vec<Monitor> = rows
		.iter()
		.map(|row| {
			let monitor: String = row.get(0);
			serde_json::from_str(&monitor).unwrap()
		})
		.filter(|monitor: &Monitor| monitor.is_overdue())
		.collect();
	Ok(Monitors::from(monitors))
}

pub async fn check_for_duplicate_monitor(
	db: &mut sqlx::Transaction<'_, sqlx::Any>,
	monitor: &Monitor,
	model_id: Id,
) -> Result<bool> {
	// Pull all rows with matching metric and cadence
	// Verify none of them have identical thresholds
	let rows = sqlx::query(
		"
			select
				data
			from
				monitors
			where
				model_id = $1
		",
	)
	.bind(model_id.to_string())
	.fetch_all(db)
	.await?;
	let result = rows
		.iter()
		.map(|row| {
			let monitor_json: String = row.get(0);
			let monitor: Monitor =
				serde_json::from_str(&monitor_json).expect("Could not parse stored alert");
			monitor
		})
		.any(|el| el.cadence == monitor.cadence && el.threshold == monitor.threshold);

	Ok(result)
}

pub async fn create_monitor(
	db: &mut sqlx::Transaction<'_, sqlx::Any>,
	monitor: Monitor,
	model_id: Id,
) -> Result<()> {
	let monitor_json = serde_json::to_string(&monitor)?;
	sqlx::query(
		"
			insert into monitors
				(id, model_id, data, cadence, date)
			values
				($1, $2, $3, $4, $5)
		",
	)
	.bind(monitor.id.to_string())
	.bind(model_id.to_string())
	.bind(monitor_json)
	.bind(monitor.cadence.to_string()) // FIXME cadence Encode
	.bind(time::OffsetDateTime::now_utc().unix_timestamp().to_string())
	.execute(db)
	.await?;
	Ok(())
}

pub async fn delete_monitor(
	db: &mut sqlx::Transaction<'_, sqlx::Any>,
	monitor_id: &str,
) -> Result<()> {
	sqlx::query(
		"
			delete from monitors
			where id = $1
		",
	)
	.bind(monitor_id.to_string())
	.execute(db)
	.await?;
	Ok(())
}

pub async fn update_monitor(
	db: &mut sqlx::Transaction<'_, sqlx::Any>,
	new_monitor: &Monitor,
	monitor_id: Id,
) -> Result<()> {
	let monitor_json = serde_json::to_string(new_monitor)?;
	sqlx::query(
		"
			update
				monitors
			set data = $1, date = $2
			where id = $3
		",
	)
	.bind(monitor_json)
	.bind(time::OffsetDateTime::now_utc().unix_timestamp().to_string())
	.bind(monitor_id.to_string())
	.execute(db)
	.await?;
	Ok(())
}

/// Read the model, find the training metric value for the given AlertMetric
pub async fn find_current_data(
	metric: AlertMetric,
	model_id: Id,
	app_state: &AppState,
) -> Result<f32> {
	// Grab the model from the DB
	let bytes = get_model_bytes(&app_state.storage, model_id).await?;
	let model = tangram_model::from_bytes(&bytes)?;
	// Determine model type
	let model_inner = model.inner();
	let model_type = AlertModelType::from(model_inner);
	if !metric.validate(model_type) {
		return Err(anyhow!(
			"Invalid metric {} for model type {:?}",
			metric,
			model_type
		));
	}

	// match on model_inner
	let result = match metric {
		AlertMetric::Accuracy => {
			// We know we have a classifier, need to get the accuracy from either type
			match model_inner {
				tangram_model::ModelInnerReader::BinaryClassifier(binary_classifier) => {
					binary_classifier
						.read()
						.test_metrics()
						.default_threshold()
						.accuracy()
				}
				tangram_model::ModelInnerReader::MulticlassClassifier(multiclass_classifier) => {
					multiclass_classifier.read().test_metrics().accuracy()
				}
				_ => unreachable!(),
			}
		}
		AlertMetric::MeanSquaredError => {
			// we know we have a regressor, just read it
			match model_inner {
				tangram_model::ModelInnerReader::Regressor(regressor) => {
					regressor.read().test_metrics().mse()
				}
				_ => unreachable!(),
			}
		}
		AlertMetric::RootMeanSquaredError => {
			// we know we have a regressor, just read it
			match model_inner {
				tangram_model::ModelInnerReader::Regressor(regressor) => {
					regressor.read().test_metrics().rmse()
				}
				_ => unreachable!(),
			}
		}
	};
	Ok(result)
}

/// Manage periodic alerting
pub async fn alert_manager(app: Arc<AppState>, notify: Arc<Notify>) -> Result<()> {
	// TODO - webhook check.
	// Scrape the DB for any incomplete webhook attempts, and spawn an exponential decay thread for any found

	let (begin, period) = if cfg!(not(debug_assertions)) {
		// In release mode, calculate time until next heartbeat
		// Start heartbeat at the top of the hour, run once per hour
		// FIXME - see https://github.com/tangramdotdev/tangram/blob/879c3805e81238e4c30c26725e1bdca5cd0d095e/crates/app/routes/track/server/post.rs#L231
		// that uses chrono, do the same thing with time
		let now = time::OffsetDateTime::now_utc();
		let now_timestamp = now.unix_timestamp();
		let hour = now.hour();
		let next_start = now.replace_time(time::Time::from_hms(hour + 1, 0, 0)?);
		let next_start_timestamp = next_start.unix_timestamp();
		let now_instant = tokio::time::Instant::now();
		let delay = tokio::time::Duration::from_secs(
			(next_start_timestamp - now_timestamp).try_into().unwrap(),
		);
		(
			now_instant + delay,
			ALERT_METRICS_HEARTBEAT_DURATION_PRODUCTION,
		)
	} else {
		// In every mode other than release, don't introduce a delay.
		(
			tokio::time::Instant::now(),
			ALERT_METRICS_HEARTBEAT_DURATION_TESTING,
		)
	};
	// start interval.
	let mut interval = tokio::time::interval_at(begin, period);

	// Each interval:
	loop {
		// If we've been notified of shutdown, break the loop.  Otherwise, continue and handle interval
		select! {
			_ = interval.tick().fuse() => {},
			_ = notify.notified().fuse() => break,
		}
		// Grab all currently enabled alerts
		let enabled = get_overdue_monitors(&app).await?;
		// For each alert:
		for monitor in enabled.alerts() {
			handle_monitor(&app, monitor).await?;
		}
	}
	Ok(())
}

async fn handle_monitor(app: &AppState, monitor: &Monitor) -> Result<()> {
	let minimum_metrics_threshold = if cfg!(not(debug_assertions)) {
		ALERT_METRICS_MINIMUM_PRODUCTION_METRICS_THRESHOLD
	} else {
		ALERT_METRICS_MINIMUM_PRODUCTION_METRICS_DEBUG_THRESHOLD
	};

	let mut txn = app.begin_transaction().await?;
	let not_enough_existing_metrics =
		get_total_production_metrics(txn.borrow_mut()).await? < minimum_metrics_threshold;
	if not_enough_existing_metrics {
		return Ok(());
	}
	app.commit_transaction(txn).await?;

	let results = check_metrics(monitor, app).await?;
	let exceeded_thresholds: Vec<&AlertResult> = results
		.iter()
		.filter(|r| {
			let (upper, lower) = monitor.get_thresholds();
			let upper_exceeded = if let Some(u) = upper {
				r.observed_variance > u
			} else {
				false
			};
			let lower_exceeded = if let Some(l) = lower {
				r.observed_variance < l
			} else {
				false
			};
			upper_exceeded || lower_exceeded
		})
		.collect();
	push_alerts(&exceeded_thresholds, &monitor.methods, app).await?;

	let alert_data = AlertData {
		id: Id::generate(),
		monitor: monitor.to_owned(),
		results: results.to_owned(),
		timestamp: time::OffsetDateTime::now_utc().unix_timestamp(),
	};

	let mut txn = app.begin_transaction().await?;
	write_alert(alert_data, monitor.id, txn.borrow_mut()).await?;
	app.commit_transaction(txn).await?;

	Ok(())
}

/// Return the current observed values for each heuristic
async fn check_metrics(monitor: &Monitor, app_state: &AppState) -> Result<Vec<AlertResult>> {
	let mut results = Vec::new();
	let current_training_value =
		find_current_data(monitor.threshold.metric, monitor.model_id, app_state).await?;
	dbg!(current_training_value);
	let mut txn = app_state.begin_transaction().await?;
	let current_production_value =
		get_production_metric(monitor.threshold.metric, monitor.model_id, txn.borrow_mut()).await?;
	dbg!(current_production_value);
	if current_production_value.is_none() {
		return Err(anyhow!("Unable to find production metric value"));
	}
	let current_production_value = current_production_value.unwrap();
	let observed_variance = match monitor.threshold.mode {
		MonitorThresholdMode::Absolute => current_training_value - current_production_value,
		MonitorThresholdMode::Percentage => ((current_training_value - current_production_value) / current_training_value) * 100.0
	};
	results.push(AlertResult {
		metric: monitor.threshold.metric,
		observed_value: current_training_value,
		observed_variance,
	});
	app_state.commit_transaction(txn).await?;
	Ok(results)
}

/// Retrieve the latest value for the given metric from the production_metrics table
pub async fn get_production_metric(
	metric: AlertMetric,
	model_id: Id,
	txn: &mut sqlx::Transaction<'_, sqlx::Any>,
) -> Result<Option<f32>> {
	let row = sqlx::query(
		"
			select
				data
			from
				production_metrics
			where
				model_id = $1
			order by
				hour
			desc
			limit 1
		",
	)
	.bind(model_id.to_string())
	.fetch_optional(txn.borrow_mut())
	.await?;
	if let Some(row) = row {
		let data: String = row.get(0);
		let production_metrics: ProductionMetrics = serde_json::from_str(&data)?;
		let output = production_metrics.finalize();
		let metrics = output.prediction_metrics;
		if let Some(metrics) = metrics {
			use ProductionPredictionMetricsOutput::*;
			match metrics {
				Regression(r) => match metric {
					AlertMetric::MeanSquaredError => Ok(Some(r.mse)),
					AlertMetric::RootMeanSquaredError => Ok(Some(r.rmse)),
					_ => Ok(None),
				},
				BinaryClassification(bc) => match metric {
					AlertMetric::Accuracy => Ok(Some(bc.accuracy)),
					_ => Ok(None),
				},
				MulticlassClassification(mc) => match metric {
					AlertMetric::Accuracy => Ok(Some(mc.accuracy)),
					_ => Ok(None),
				},
			}
		} else {
			Ok(None)
		}
	} else {
		Ok(None)
	}
}

async fn get_total_production_metrics(txn: &mut sqlx::Transaction<'_, sqlx::Any>) -> Result<i64> {
	let result = sqlx::query(
		"
			select
				count(*)
			from production_metrics
		",
	)
	.fetch_one(txn.borrow_mut())
	.await?;
	let result: i64 = result.get(0);
	Ok(result)
}

/// Send alerts containing all exceeded thresholds to each enabled alert method
async fn push_alerts(
	exceeded_thresholds: &[&AlertResult],
	methods: &[AlertMethod],
	context: &AppState,
) -> Result<()> {
	for method in methods {
		method.send_alert(exceeded_thresholds, context).await?;
	}
	Ok(())
}

/// Log the completion of an alert handling process
async fn write_alert(
	alert_data: AlertData,
	monitor_id: Id,
	txn: &mut sqlx::Transaction<'_, sqlx::Any>,
) -> Result<()> {
	let mut db = txn.begin().await?;
	let data = serde_json::to_string(&alert_data)?;

	sqlx::query(
		"
		 insert into alerts
		 	(id, monitor_id, data, date)
		 values
		 	($1, $2, $3, $4)
		",
	)
	.bind(alert_data.id.to_string())
	.bind(monitor_id.to_string())
	.bind(data)
	.bind(alert_data.timestamp)
	.execute(db.borrow_mut())
	.await?;
	db.commit().await?;
	Ok(())
}

impl App {
	/// Retrieve all recorded alerts across all monitors for a given model
	pub async fn get_all_alerts_for_model(
		&self,
		txn: &mut sqlx::Transaction<'_, sqlx::Any>,
		model_id: Id,
	) -> Result<Vec<AlertData>> {
		let rows = sqlx::query(
			"
			select
				alerts.data
			from
				monitors
			join
				alerts
			on
				monitors.id = alerts.monitor_id
			where 
				monitors.model_id = $1
		",
		)
		.bind(model_id.to_string())
		.fetch_all(txn.borrow_mut())
		.await?;
		let result = rows
			.iter()
			.map(|row| {
				let data: String = row.get(0);
				let data: AlertData =
					serde_json::from_str(&data).expect("Found malformed alert data");
				data
			})
			.collect();
		Ok(result)
	}

	/// Retrieve a specific alert by id
	pub async fn get_alert(
		&self,
		txn: &mut sqlx::Transaction<'_, sqlx::Any>,
		alert_id: Id,
	) -> Result<Option<AlertData>> {
		let result = sqlx::query(
			"
				select
					data
				from
					alerts
				where
					id = $1
			",
		)
		.bind(alert_id.to_string())
		.fetch_optional(txn.borrow_mut())
		.await?;

		if let Some(row) = result {
			let data: String = row.get(0);
			let data: AlertData = serde_json::from_str(&data).expect("Malformed alert data");
			Ok(Some(data))
		} else {
			Ok(None)
		}
	}
}
