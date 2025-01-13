
//-----------------------TimeFrme---------------: START
#[derive(Debug, Clone, Copy)]
pub enum TimeFrame {
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
    FourHours,
    OneDay
}
impl  TimeFrame {
    pub fn from_str(s: &str) -> Option<TimeFrame> {
        match s {
            "1min" => Some(TimeFrame::OneMinute),
            "5min" => Some(TimeFrame::FiveMinutes),
            "15min" => Some(TimeFrame::FifteenMinutes),
            "30min" => Some(TimeFrame::ThirtyMinutes),
            "1hour" => Some(TimeFrame::OneHour),
            "4hour" => Some(TimeFrame::FourHours),
            "1day" => Some(TimeFrame::OneDay),
            _ => None,
        }
    }

    pub fn to_str(&self) -> &str {
        match self {
            TimeFrame::OneMinute => "1min",
            TimeFrame::FiveMinutes => "5min",
            TimeFrame::FifteenMinutes => "15min",
            TimeFrame::ThirtyMinutes => "30min",
            TimeFrame::OneHour => "1hour",
            TimeFrame::FourHours => "4hour",
            TimeFrame::OneDay => "1day",
        }
    }
}

//-----------------------TimeFrme---------------: END

//-----------------------DateTime---------------: START
#[derive(Debug, Clone)]
pub struct DateTime {
    pub year: String,
    pub month: String,
    pub day: String,
    pub hour: Option<String>,
    pub minute: Option<String>,
    pub second: Option<String>,
    pub millisecond: Option<String>,
}

impl DateTime {
    pub fn from_slice<S, I>(s: I) -> Option<DateTime>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let parts = s
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect::<Vec<_>>();
        let mut parts = parts.iter().map(|s| s.as_str());
        let year = parts.next()?.to_string();
        let month = parts.next()?.to_string();
        let day = parts.next()?.to_string();
        let hour = parts.next().map(|s| s.to_string());
        let minute = parts.next().map(|s| s.to_string());
        let second = parts.next().map(|s| s.to_string());
        let millisecond = parts.next().map(|s| s.to_string());

        if year.len() != 4 || month.len() != 2 || day.len() != 2 || hour.is_none() || minute.is_none() || second.is_none() || millisecond.is_none() {
            return None;
        }
        Some(DateTime { year, month, day, hour, minute, second, millisecond })
    }

    pub fn from_str(s: &str) -> Result<DateTime, Box<dyn std::error::Error>> {
        let mut parts = s.split(' ');
        let date_part = parts.next().ok_or("Empty date string.")?;
        let time_part = parts.next().unwrap_or("");

        let date_time_parts: Vec<&str> = date_part.split('-').chain(time_part.split(':')).collect();
        let r = DateTime::from_slice(date_time_parts);
        match r {
            Some(r) => Ok(r),
            None => Err("Invalid date time format. Use `YYYY-MM-DD HH:MM:SS.MS`".into()),
        }
    }

    pub fn to_string(&self) -> String {
        let mut s = format!("{}-{}-{}", self.year, self.month, self.day);
        if let Some(hour) = &self.hour {
            s = format!("{} {}", s, hour);
        }
        if let Some(minute) = &self.minute {
            s = format!("{}:{}", s, minute);
        }
        if let Some(second) = &self.second {
            s = format!("{}:{}", s, second);
        }
        if let Some(millisecond) = &self.millisecond {
            s = format!("{}:{}", s, millisecond);
        }
        s
    }
}

//------------------DateTime---------------------: END


//-----------------------FetchType---------------: START
#[derive(Debug, Clone, Copy)]
pub enum FetchType {
    Quote,
    Financial,
    Profile,
    Rating,
    Outlook,
    History,
    DividendHistory,
    SplitHistory,
    IntraDay,
    Daily,
}
impl FetchType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "quote" => FetchType::Quote,
            "financial" => FetchType::Financial,
            "profile" => FetchType::Profile,
            "rating" => FetchType::Rating,
            "outlook" => FetchType::Outlook,
            "history" => FetchType::History,
            "dividend_history" => FetchType::DividendHistory,
            "split_history" => FetchType::SplitHistory,
            "intraday" => FetchType::IntraDay,
            "daily" => FetchType::Daily,
            _ => unreachable!(),
        }
    }
}

//------------------FetchType---------------------: END

//-----------------------Either---------------: START
/// Helper enum to handle single value or array of values.
#[derive(Debug, Clone, Copy)]
pub enum Either<L, R> {
    Single(L),
    Array(R),
}
impl Either<String, Vec<String>> 
where
    String: Clone,
    Vec<String>: Clone,
{
    pub fn into_iter(self) -> Box<dyn Iterator<Item = String>> {
        match self {
            Either::Single(symbol) => Box::new(std::iter::once(symbol)),
            Either::Array(symbols) => Box::new(symbols.into_iter()),
        }
    }

    pub fn from_str(input: &str) -> Either<String, Vec<String>>{
        let parts: Vec<&str> = input.split(',').map(|s| s.trim()).collect();
        if parts.len() == 1 {
            Either::Single(parts[0].to_string())
        } else {
            Either::Array(parts.into_iter().map(|s| s.to_string()).collect())
        }
    }

    pub fn from_vec(input: Vec<String>) -> Either<String, Vec<String>> {
        if input.len() == 1 {
            Either::Single(input[0].clone())
        } else {
            Either::Array(input)
        }
    }
}

//------------------Either---------------------: END

