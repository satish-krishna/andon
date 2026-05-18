use anyhow::Result;
use minijinja::{Environment, context};

use super::assets;
use super::model::ReportData;

const TEMPLATE_SRC: &str = include_str!("../../templates/session_report.html.j2");

pub fn render(data: &ReportData) -> Result<String> {
    let mut env = Environment::new();
    env.add_template("report", TEMPLATE_SRC)?;
    let tpl = env.get_template("report")?;
    let report_data_json = serde_json::to_string(data)?;
    let html = tpl.render(context! {
        data            => data,
        report_data_json => report_data_json,
        chart_js        => assets::CHART_JS,
        css             => assets::REPORT_CSS,
    })?;
    Ok(html)
}
