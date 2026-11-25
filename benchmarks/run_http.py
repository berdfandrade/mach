"""Paired localhost HTTP benchmark. Owns its PostgreSQL cluster and both servers."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import math
import os
from pathlib import Path
import platform
import secrets
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ("plaintext", "GET", "/bench/plain", None),
    ("json", "GET", "/bench/json", None),
    ("parameter", "GET", "/bench/path/19", None),
    ("validation", "POST", "/bench/validate", b'{"name":"example"}'),
    ("authenticated", "GET", "/bench/auth", None),
    ("read", "GET", "/bench/records/1", None),
    ("pagination", "GET", "/bench/records?limit=10", None),
    ("write", "POST", "/bench/records", b'{"name":"example"}'),
]


def cpu(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().split()
    return (int(fields[13]) + int(fields[14])) / os.sysconf("SC_CLK_TCK")


def trial(port, pid, case, headers, n, workers):
    label, method, path, body = case

    def batch(count):
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
        times = []
        try:
            for _ in range(count):
                start = time.perf_counter_ns()
                conn.request(method, path, body, headers)
                res = conn.getresponse()
                res.read()
                if res.status not in (200, 204):
                    raise RuntimeError(f"{label}: {res.status}")
                times.append((time.perf_counter_ns() - start) / 1e6)
            return times
        finally:
            conn.close()

    batch(10)
    start = time.perf_counter()
    cpu_start = cpu(pid)
    with ThreadPoolExecutor(workers) as pool:
        samples = sorted(v for chunk in pool.map(batch, [n//workers + (i < n%workers) for i in range(workers)]) for v in chunk)
    seconds = time.perf_counter() - start
    rss = next(s for s in Path(f"/proc/{pid}/status").read_text().splitlines() if s.startswith("VmHWM:"))
    metrics = dict(workload=label, concurrency=workers, requests=n, rps=n/seconds,
                   server_cpu_pct=100*(cpu(pid)-cpu_start)/seconds, peak_rss_kib=int(rss.split()[1]))
    for p in ([50, 95, 99, 99.9] if n >= 10000 else [50, 95, 99]):
        metrics[f"p{p}_ms"] = samples[math.ceil(n*p/100)-1]
    return metrics


def run(root, args):
    pg = Path(subprocess.check_output(["pg_config","--bindir"], text=True).strip())
    data = root / "pgdata"
    subprocess.run([pg/"initdb","-D",data,"-A","trust","-U","postgres","--no-locale"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run([pg/"pg_ctl","-D",data,"-l",root/"postgres.log","-o",f"-k {root} -h ''","start"],check=True, stdout=subprocess.DEVNULL)
    try:
        env = dict(os.environ, PGHOST=str(root), PGUSER="postgres", PGDATABASE="postgres")
        token = secrets.token_urlsafe(32)
        env.update(DATABASE_URL=f"postgresql://postgres@localhost/postgres?host={root}", MACH_TOKEN=token,
                   MACH_GRANTS="records:read:1,records:list:collection,records:create:collection")
        report = {"platform":platform.platform(), "cpu":platform.machine(), "logical_cpus":os.cpu_count(),
                  "rustc":subprocess.check_output(["rustc","-V"],text=True).strip(),
                  "postgres":subprocess.check_output([pg/"postgres","--version"],text=True).strip(),"runs":[]}
        for repeat in range(args.repeats):
            servers = ["mach-server", "axum-server"]
            for name in servers[::(-1 if repeat % 2 else 1)]:
                subprocess.run([pg/"psql","-qX","-v","ON_ERROR_STOP=1"],env=env,input="""
                    DROP TABLE IF EXISTS records;
                    CREATE TABLE records (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                          name text NOT NULL CHECK (char_length(name) BETWEEN 2 AND 100));
                    INSERT INTO records (name) SELECT 'record' FROM generate_series(1,100);
                """,text=True,check=True,stderr=subprocess.DEVNULL)
                with socket.socket() as s:
                    s.bind(("127.0.0.1",0))
                    port = s.getsockname()[1]
                binary = Path(os.environ.get("CARGO_TARGET_DIR",ROOT/"target"))/"release"/name
                started = time.perf_counter()
                proc = subprocess.Popen([binary],env=dict(env,BENCH_BIND=f"127.0.0.1:{port}"))
                try:
                    for _ in range(500):
                        if proc.poll() is not None:
                            raise RuntimeError("server exited during startup")
                        conn = http.client.HTTPConnection("127.0.0.1",port,timeout=1)
                        try:
                            conn.request("GET","/ready")
                            if conn.getresponse().status == 200:
                                break
                        except OSError:
                            time.sleep(.005)
                        finally:
                            conn.close()
                    else:
                        raise RuntimeError("server readiness timeout")
                    result = dict(server=name,repeat=repeat,binary_bytes=binary.stat().st_size,
                                  startup_ms=1000*(time.perf_counter()-started),cases=[])
                    headers = {"Content-Type":"application/json","Authorization":"Bearer "+token}
                    for concurrency in args.concurrency:
                        for case in CASES:
                            result["cases"].append(trial(port,proc.pid,case,headers,args.requests,concurrency))
                    report["runs"].append(result)
                finally:
                    proc.terminate()
                    proc.wait(timeout=20)
        print(json.dumps(report,separators=(",",":")))
    finally:
        subprocess.run([pg/"pg_ctl","-D",data,"-m","fast","stop"],check=True,stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--requests",type=int,default=10000)
    p.add_argument("--concurrency",nargs="+",type=int,default=[1,16])
    p.add_argument("--repeats",type=int,default=3)
    args=p.parse_args()
    if min([args.requests,args.repeats,*args.concurrency])<1 or max(args.concurrency)>args.requests:
        p.error("choose positive counts with concurrency <= requests")
    subprocess.run(["cargo","build","--release","--locked","-p","mach-bench"],cwd=ROOT,check=True)
    with tempfile.TemporaryDirectory(prefix="mach-bench-") as tmp:
        run(Path(tmp),args)
