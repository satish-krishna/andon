"""Send a synthetic OTLP HTTP/protobuf payload to verify ingestion."""
import struct
import time
import urllib.request

# Hand-rolled minimal protobuf — easier than pulling opentelemetry-proto deps.
def varint(v):
    out = b""
    while v > 0x7F:
        out += bytes([(v & 0x7F) | 0x80])
        v >>= 7
    out += bytes([v])
    return out

def tag(field, wire):
    return varint((field << 3) | wire)

def length_delim(field, data):
    return tag(field, 2) + varint(len(data)) + data

def string(field, s):
    return length_delim(field, s.encode("utf-8"))

def fixed64(field, v):
    return tag(field, 1) + struct.pack("<Q", v)

def int_value(field, v):
    return tag(field, 0) + varint(v if v >= 0 else v + (1 << 64))

def any_value_string(s):
    # AnyValue { string_value = 1, ... }
    return string(1, s)

def keyvalue(k, v_any):
    # KeyValue { key=1, value=2 (AnyValue) }
    return string(1, k) + length_delim(2, v_any)

def resource(attrs):
    body = b""
    for k, v in attrs:
        body += length_delim(1, keyvalue(k, any_value_string(v)))
    return body

def number_data_point(attrs, ts_ns, as_int):
    # NumberDataPoint { attributes=7, time_unix_nano=3 (fixed64),
    #                   as_int=6 (fixed64), as_double=4 }
    body = b""
    for k, v in attrs:
        body += length_delim(7, keyvalue(k, any_value_string(v)))
    body += fixed64(3, ts_ns)
    body += fixed64(6, as_int if as_int >= 0 else as_int + (1 << 64))
    return body

def sum_metric(data_points, is_monotonic=True):
    # Sum { data_points=1, aggregation_temporality=2, is_monotonic=3 }
    body = b""
    for dp in data_points:
        body += length_delim(1, dp)
    body += int_value(2, 2)  # CUMULATIVE
    body += int_value(3, 1 if is_monotonic else 0)
    return body

def metric(name, sum_body):
    # Metric { name=1, sum=7 }
    return string(1, name) + length_delim(7, sum_body)

def scope_metrics(metrics):
    body = b""
    for m in metrics:
        body += length_delim(2, m)  # metrics = field 2
    return body

def resource_metrics(res, scope_metrics_body):
    # ResourceMetrics { resource=1, scope_metrics=2 }
    body = length_delim(1, res) + length_delim(2, scope_metrics_body)
    return body

def export_request(rm):
    # ExportMetricsServiceRequest { resource_metrics=1 }
    return length_delim(1, rm)


def main():
    now_ns = int(time.time() * 1_000_000_000)
    res = resource([
        ("service.name", "claude-code"),
        ("service.version", "1.0.0-smoke"),
        ("session.id", "smoke-session-001"),
        ("user.account_uuid", "smoke-user"),
        ("host.arch", "amd64"),
        ("os.type", "windows"),
    ])

    cost_dp = number_data_point(
        [("model", "claude-sonnet-4-6")], now_ns, 0
    )
    # cost is a double — re-emit with as_double
    cost_dp_d = b""
    for k, v in [("model", "claude-sonnet-4-6")]:
        cost_dp_d += length_delim(7, keyvalue(k, any_value_string(v)))
    cost_dp_d += fixed64(3, now_ns)
    cost_dp_d += tag(4, 1) + struct.pack("<d", 0.4242)  # as_double = field 4

    cost_metric = metric("claude_code.cost.usage", sum_metric([cost_dp_d]))

    token_dp = number_data_point(
        [("type", "input"), ("model", "claude-sonnet-4-6")], now_ns, 12345
    )
    token_metric = metric("claude_code.token.usage", sum_metric([token_dp]))

    token_dp2 = number_data_point(
        [("type", "output"), ("model", "claude-sonnet-4-6")], now_ns, 6789
    )
    token_metric2 = metric("claude_code.token.usage", sum_metric([token_dp2]))

    decision_dp = number_data_point(
        [("tool", "Edit"), ("decision", "accept"), ("language", "rust")], now_ns, 1
    )
    decision_metric = metric("claude_code.code_edit_tool.decision", sum_metric([decision_dp]))

    decision_dp2 = number_data_point(
        [("tool", "Edit"), ("decision", "reject"), ("language", "rust")], now_ns, 1
    )
    decision_metric2 = metric("claude_code.code_edit_tool.decision", sum_metric([decision_dp2]))

    decision_dp3 = number_data_point(
        [("tool", "Edit"), ("decision", "accept"), ("language", "typescript")], now_ns, 1
    )
    decision_metric3 = metric("claude_code.code_edit_tool.decision", sum_metric([decision_dp3]))

    active_dp = b""
    for k, v in [("type", "user")]:
        active_dp += length_delim(7, keyvalue(k, any_value_string(v)))
    active_dp += fixed64(3, now_ns)
    active_dp += tag(4, 1) + struct.pack("<d", 600.0)  # 10 minutes user
    active_metric = metric("claude_code.active_time.total", sum_metric([active_dp]))

    sm = scope_metrics([cost_metric, token_metric, token_metric2, decision_metric, decision_metric2, decision_metric3, active_metric])
    rm = resource_metrics(res, sm)
    body = export_request(rm)

    req = urllib.request.Request(
        "http://127.0.0.1:4318/v1/metrics",
        data=body,
        method="POST",
        headers={"Content-Type": "application/x-protobuf"},
    )
    with urllib.request.urlopen(req, timeout=5) as resp:
        print("POST /v1/metrics:", resp.status, resp.read())


if __name__ == "__main__":
    main()
