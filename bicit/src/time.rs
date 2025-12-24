use chrono::Duration;

pub fn get_hhmmss(duration: Duration) -> String {
    let totsec = duration.num_seconds();
    let seconds = totsec % 60;
    let minutes = (totsec / 60) % 60;
    let hours = (totsec / 60) / 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
