use chrono::{DateTime, Datelike, Timelike, Utc};
use log::{info, warn, error};
use proxy_wasm::{traits::*, types::*};
use serde::Deserialize;
use serde_json_wasm::de;
use std::collections::HashMap;
use std::convert::TryInto;
use std::cmp::max;
use phf;

// -----------------------------------------------------------------------------
// Config
// -----------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
struct Config {
    #[serde(default = "default_negative_1")]
    second: i32,
    #[serde(default = "default_negative_1")]
    minute: i32,
    #[serde(default = "default_negative_1")]
    hour: i32,
    #[serde(default = "default_negative_1")]
    day: i32,
    #[serde(default = "default_negative_1")]
    month: i32,
    #[serde(default = "default_negative_1")]
    year: i32,
    #[serde(default = "default_limit_by")]
    limit_by: String,
    #[serde(default = "default_empty")]
    header_name: String,
    #[serde(default = "default_empty")]
    path: String,
    #[serde(default = "default_policy")]
    policy: String,
    #[serde(default = "default_true")]
    fault_tolerant: bool,
    #[serde(default = "default_false")]
    hide_client_headers: bool,
    #[serde(default = "default_429")]
    error_code: u32,
    #[serde(default = "default_msg")]
    error_message: String,
}

fn default_negative_1() -> i32 {
    -1
}

fn default_empty() -> String {
    "".to_string()
}

fn default_limit_by() -> String {
    "ip".to_string()
}

fn default_policy() -> String {
    "local".to_string()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_429() -> u32 {
    429
}

fn default_msg() -> String {
    "Rust informs: API rate limit exceeded!".to_string()
}

// -----------------------------------------------------------------------------
// Timestamps
// -----------------------------------------------------------------------------

type TimestampMap = HashMap<&'static str, i64>;

fn get_timestamps(now: DateTime<Utc>) -> TimestampMap {
    let mut ts = TimestampMap::new();

    ts.insert("now", now.timestamp());

    let second = now.with_nanosecond(0).unwrap();
    ts.insert("second", second.timestamp());

    let minute = second.with_second(0).unwrap();
    ts.insert("minute", minute.timestamp());

    let hour = minute.with_minute(0).unwrap();
    ts.insert("hour", hour.timestamp());

    let day = hour.with_hour(0).unwrap();
    ts.insert("day", day.timestamp());

    let month = day.with_day(1).unwrap();
    ts.insert("month", month.timestamp());

    let year = month.with_month(1).unwrap();
    ts.insert("year", year.timestamp());

    ts
}

// -----------------------------------------------------------------------------
// Root Context
// -----------------------------------------------------------------------------

static EXPIRATION: phf::Map<&'static str, i32> = phf::phf_map! {
    "second" => 1,
    "minute" => 60,
    "hour" => 3600,
    "day" => 86400,
    "month" => 2592000,
    "year" => 31536000,
};

static X_RATE_LIMIT_LIMIT: phf::Map<&'static str, &'static str> = phf::phf_map! {
    "second" => "X-RateLimit-Limit-Second",
    "minute" => "X-RateLimit-Limit-Minute",
    "hour" => "X-RateLimit-Limit-Hour",
    "day" => "X-RateLimit-Limit-Day",
    "month" => "X-RateLimit-Limit-Month",
    "year" => "X-RateLimit-Limit-Year",
};

static X_RATE_LIMIT_REMAINING: phf::Map<&'static str, &'static str> = phf::phf_map! {
    "second" => "X-RateLimit-Remaining-Second",
    "minute" => "X-RateLimit-Remaining-Minute",
    "hour" => "X-RateLimit-Remaining-Hour",
    "day" => "X-RateLimit-Remaining-Day",
    "month" => "X-RateLimit-Remaining-Month",
    "year" => "X-RateLimit-Remaining-Year",
};

proxy_wasm::main! {{

    proxy_wasm::set_log_level(LogLevel::Debug);
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> {
        Box::new(RateLimitingRoot {
            config: None,
        })
    });
}}

struct RateLimitingRoot {
    config: Option<Config>,
}

impl Context for RateLimitingRoot {}
impl RootContext for RateLimitingRoot {
    fn get_type(&self) -> Option<ContextType> {
        Some(ContextType::HttpContext)
    }

    fn on_configure(&mut self, config_size: usize) -> bool {
        info!("on_configure: config_size: {}", config_size);

        if let Some(config_bytes) = self.get_plugin_configuration() {
            // assert!(config_bytes.len() == config_size);

            match de::from_slice::<Config>(&config_bytes) {
                Ok(config) => {

                    if config.policy != "local" {
                        error!("on_configure: only the local policy is supported for now");
                        return false;
                    }

                    self.config = Some(config);

                    info!("on_configure: loaded configuration: {:?}", self.config);
                    true
                }
                Err(err) => {
                    warn!(
                        "on_configure: failed parsing configuration: {}: {}",
                        String::from_utf8(config_bytes).unwrap(),
                        err
                    );
                    false
                }
            }
        } else {
            warn!("on_configure: failed getting configuration");
            false
        }
    }

    fn create_http_context(&self, context_id: u32) -> Option<Box<dyn HttpContext>> {
        info!("create_http_context: configuration: context_id: {} | {:?}", context_id, self.config);
        if let Some(config) = &self.config {
            let mut limits = HashMap::<&'static str, i32>::new();

            limits.insert("second", config.second);
            limits.insert("minute", config.minute);
            limits.insert("hour", config.hour);
            limits.insert("day", config.day);
            limits.insert("month", config.month);
            limits.insert("year", config.year);

            Some(Box::new(RateLimitingHttp {
                _context_id: context_id,
                config: config.clone(),
                limits: limits,
                headers: None,
            }))
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// Plugin Context
// -----------------------------------------------------------------------------

struct RateLimitingHttp {
    _context_id: u32,
    config: Config,
    limits: HashMap<&'static str, i32>,
    headers: Option<HashMap<&'static str, String>>,
}

struct Usage {
    limit: i32,
    remaining: i32,
    usage: i32,
    cas: Option<u32>,
}

type UsageMap = HashMap<&'static str, Usage>;

#[derive(Default)]
struct Usages {
    counters: Option<UsageMap>,
    stop: Option<&'static str>,
    err: Option<String>,
}

trait RateLimitingPolicy {
    fn usage(&self, id: &str, period: &'static str, ts: &TimestampMap) -> Result<(i32, Option<u32>), String>;

    fn increment(&mut self, id: &str, counters: &UsageMap, ts: &TimestampMap);
}

// Local policy implementation:
impl RateLimitingPolicy for RateLimitingHttp {
    fn usage(&self, id: &str, period: &'static str, ts: &TimestampMap) -> Result<(i32, Option<u32>), String> {
        let cache_key = self.get_local_key(id, period, ts[period]);
        match self.get_shared_data(&cache_key) {
            (Some(data), cas) => {
                Ok((i32::from_le_bytes(data.try_into().unwrap_or_else(|_| [0, 0, 0, 0])), cas))
            }
            (None, cas) => {
                Ok((0, cas))
            }
            // proxy-wasm-rust-sdk panics on errors and converts
            // Status::NotFound to (None, cas),
            // so this function never returns Err
        }
    }

    fn increment(&mut self, id: &str, counters: &UsageMap, ts: &TimestampMap) {
        for (period, usage) in counters {
            let cache_key = self.get_local_key(id, period, ts[period]);
            let mut value = usage.usage;
            let mut cas = usage.cas;

            let mut saved = false;
            for _ in 0..10 {
                let buf = (value + 1).to_le_bytes();
                match self.set_shared_data(&cache_key, Some(&buf), cas) {
                    Ok(()) => {
                        saved = true;
                        break;
                    }
                    Err(Status::CasMismatch) => {
                        if let Ok((nvalue, ncas)) = self.usage(id, period, ts) {
                            if ncas != None {
                                value = nvalue;
                                cas = ncas;
                            }
                        }
                    }
                    Err(_) => {
                        // anything else will cause proxy-wasm-rust-sdk to panic.
                    }
                }
            }

            if !saved {
                log::error!("could not increment counter for period '{}'", period)
            }
        }
    }
}

impl RateLimitingHttp {
    fn get_prop(&self, ns: &str, prop: &str) -> String {
        if let Some(addr) = self.get_property(vec![ns, prop]) {
            match std::str::from_utf8(&addr) {
                Ok(value) => value.to_string(),
                Err(_) => "".to_string(),
            }
        } else {
            "".to_string()
        }
    }

    fn get_identifier(&self) -> String {
        match self.config.limit_by.as_str() {
            "header" => {
                if let Some(header) = self.get_http_request_header(&self.config.header_name) {
                    return header.to_string();
                }
            }
            "path" => {
                if let Some(path) = self.get_http_request_header(":path") {
                    if path == self.config.path {
                        return path.to_string();
                    }
                }
            }
            &_ => {}
        }

        // "ip" is the fallback:
        return self.get_prop("ngx", "remote_addr");
    }

    fn get_local_key(&self, id: &str, period: &'static str, date: i64) -> String {
        format!("kong_wasm_rate_limiting_counters/ratelimit:{}:{}:{}:{}:{}",
            self.get_prop("kong", "route_id"),
            self.get_prop("kong", "service_id"),
            id, date, period)
    }

    fn get_usages(&mut self, id: &str, ts: &TimestampMap) -> Usages {
        let mut usages: Usages = Default::default();
        let mut counters = UsageMap::new();

        for (&period, &limit) in &self.limits {
            if limit == -1 {
                continue;
            }

            match self.usage(id, period, ts) {
                Ok((cur_usage, cas)) => {
                    // What is the current usage for the configured limit name?
                    let remaining = limit - cur_usage;

                    counters.insert(period, Usage {
                        limit: limit,
                        remaining: remaining,
                        usage: cur_usage,
                        cas: cas,
                    });

                    if remaining <= 0 {
                        usages.stop = Some(period);
                    }
                }
                Err(err) => {
                    usages.err = Some(err);
                    break;
                }
            }
        }

        usages.counters = Some(counters);
        usages
    }

    fn process_usage(&mut self, counters: &UsageMap, stop: Option<&'static str>, ts: &TimestampMap) -> Action {
        let now = ts["now"];
        let mut reset: i32 = 0;

        if !self.config.hide_client_headers {
            let mut limit: i32 = 0;
            let mut window: i32 = 0;
            let mut remaining: i32 = 0;
            let mut headers = HashMap::<&'static str, String>::new();

            for (period, usage) in counters {
                let cur_limit = usage.limit;
                let cur_window = EXPIRATION[period];
                let mut cur_remaining = usage.remaining;

                if stop == None || stop == Some(period) {
                    cur_remaining -= 1;
                }
                cur_remaining = max(0, cur_remaining);

                if (limit == 0) || (cur_remaining < remaining) || (cur_remaining == remaining && cur_window > window) {
                    limit = cur_limit;
                    window = cur_window;
                    remaining = cur_remaining;

                    reset = max(1, window - (now - ts[period]) as i32);
                }

                headers.insert(X_RATE_LIMIT_LIMIT[period], limit.to_string());
                headers.insert(X_RATE_LIMIT_REMAINING[period], remaining.to_string());
            }

            headers.insert("RateLimit-Limit", limit.to_string());
            headers.insert("RateLimit-Remaining", remaining.to_string());
            headers.insert("RateLimit-Reset", reset.to_string());

            self.headers = Some(headers);
        }

        if stop != None {
            if self.headers == None {
                self.headers = Some(HashMap::new());
            }
            if let Some(headers) = &mut self.headers {
                headers.insert("Retry-After", reset.to_string());
            }

            self.send_http_response(self.config.error_code, vec![], Some(self.config.error_message.as_bytes()));

            Action::Pause
        } else {
            Action::Continue
        }
    }
}

impl Context for RateLimitingHttp {}
impl HttpContext for RateLimitingHttp {
    fn on_http_request_headers(&mut self, _nheaders: usize, _eof: bool) -> Action {
        let now: DateTime<Utc> = self.get_current_time().into();

        let ts = get_timestamps(now);

        let id = self.get_identifier();

        let usages = self.get_usages(&id, &ts);

        if let Some(err) = usages.err {
            if !self.config.fault_tolerant {
                panic!("{}", err.to_string());
            }
            log::error!("failed to get usage: {}", err);
        }

        if let Some(counters) = usages.counters {
            let action = self.process_usage(&counters, usages.stop, &ts);
            if action != Action::Continue {
                return action;
            }

            self.increment(&id, &counters, &ts);
        }

        Action::Continue
    }

    fn on_http_response_headers(&mut self, _nheaders: usize, eof: bool) -> Action {
        if !eof {
            return Action::Continue;
        }

        if let Some(headers) = &self.headers {
            for (k, v) in headers {
                self.add_http_response_header(k, &v);
            }
        }

        Action::Continue
    }
}
