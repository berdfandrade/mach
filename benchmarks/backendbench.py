"""Aggregate observed BackendBench records without imputing missing metrics."""
import argparse
from collections import defaultdict
import json
import math
from pathlib import Path
from statistics import median

METRICS = ("input_tokens", "output_tokens", "cached_tokens", "tool_calls", "files_read",
           "files_modified", "compilation_attempts", "test_attempts", "elapsed_s")


def summarize(rows):
    groups = defaultdict(list)
    seen = set()
    for row in rows:
        key = (row["framework"], row["model"], row["task"])
        if key[0] not in {"MACH","Axum","NestJS","Spring Boot"} or type(key[2]) is not int or not 1 <= key[2] <= 10:
            raise ValueError("unknown framework or task")
        identity = (*key, row["run"])
        if identity in seen:
            raise ValueError("duplicate run")
        seen.add(identity)
        for flag in ("functional_pass", "security_pass"):
            if type(row[flag]) is not bool:
                raise ValueError(flag)
        for metric in METRICS:
            if metric == "elapsed_s" or metric in row:
                value = row[metric]
                allowed = (float,int) if metric == "elapsed_s" else (int,)
                if type(value) not in allowed or value < 0 or not math.isfinite(value):
                    raise ValueError(metric)
                if metric.endswith("_tokens") and not str(row.get("usage_source") or "").strip():
                    raise ValueError("token counts require a telemetry record reference")
        groups[key].append(row)
    out = []
    for (framework,model,task),group in sorted(groups.items()):
        result = dict(framework=framework,model=model,task=task,runs=len(group))
        for flag in ("functional_pass","security_pass"):
            result[flag+"_rate"] = sum(r[flag] for r in group)/len(group)
        for m in METRICS:
            values = [r[m] for r in group if m in r]
            if values:
                result[m] = {"n":len(values),"median":median(values)}
        out.append(result)
    return out


if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("records",type=Path)
    with p.parse_args().records.open() as f:
        print(json.dumps(summarize(json.loads(line) for line in f if line.strip()),sort_keys=True))
