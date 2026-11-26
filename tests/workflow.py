"""Exercise a new application using the installed CLI and a disposable database."""
import contextlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

BIN = str(Path(sys.argv[1]).resolve())
DB = "--source-only" not in sys.argv
if DB:
    assert os.environ.get("DATABASE_URL"), "run via the Testcontainers integration test"


def check_app(root):
    app = root / "demo"

    def cli(*args, ok=True, cwd=app):
        proc = subprocess.run([BIN, "--json", *args], cwd=cwd, capture_output=True, text=True)
        assert (proc.returncode == 0) == ok, (args, proc.stderr[-1500:])
        return proc.stdout

    def db_test(fixture):
        case = app / "tests" / (fixture + ".rs")
        case.write_bytes((Path(__file__).parent / "fixtures" / case.name).read_bytes())
        try:
            subprocess.run(["cargo","test","--quiet","--test",fixture],cwd=app,check=True)
        finally:
            case.unlink()

    cli("new", "demo", "--db", "postgres", cwd=root)
    cli("new", "demo", ok=False, cwd=root)
    registry = app / "src/modules/mod.rs"
    registry.write_text(registry.read_text().replace("declared: [", "declared: [/* WHY: local module registry */"))
    if os.name == "posix" and os.getuid() != 0:
        locked = app / "src/modules/zzlocked"
        locked.mkdir()
        locked.chmod(0o500)
        before = registry.read_bytes()
        try:
            cli("generate","resource","zzlocked","--crud",ok=False)
            assert registry.read_bytes() == before and list(locked.iterdir()) == []
            assert not (app / ".mach/journal.json").exists()
        finally:
            locked.chmod(0o700)
            locked.rmdir()
    cli("generate", "resource", "users", "--crud")
    assert "src/modules/categories/model.rs" in json.loads(cli("generate", "entity", "Category", "--dry-run"))["files"]
    assert json.loads(cli("generate", "resource", "users", "--crud"))["files"] == []
    assert "WHY: local module registry" in registry.read_text()
    model = app / "src/modules/users/model.rs"
    repo = app / "src/modules/users/repository.rs"
    example = Path(__file__).parent / "golden/users-model.rs"
    assert model.read_bytes() == example.read_bytes()
    model.write_text("// WHY: preserve café and user formatting.\n" + model.read_text())
    baseline = model.read_bytes()
    for spec in ("id:i64", "bad-name:string", "when:timestamp"):
        cli("field", "add", "User", spec, "--optional", ok=False)
    assert baseline == model.read_bytes()
    cli("generate", "migration")
    if DB:
        cli("migrate")
        db_test("history")
    cli("field", "add", "User", "phone:string", "--optional")
    before = model.read_bytes()
    report = json.loads(cli("field", "rename", "User", "name", "full_name", "--dry-run"))
    assert report["api_contract"] and before == model.read_bytes()

    custom = app / "src/modules/users/custom.rs"
    custom.write_text("pub fn label(user: &super::model::User) -> &str { &user.name }\n")
    mod = custom.with_name("mod.rs")
    previous = mod.read_bytes()
    mod.write_bytes(previous + b"pub mod custom;\n")
    cli("field", "rename", "User", "name", "full_name", ok=False)
    assert model.read_bytes() == before and b"&user.name" in custom.read_bytes()
    mod.write_bytes(previous)
    custom.unlink()

    cli("field", "rename", "User", "name", "full_name")
    assert "preserve café" in model.read_text()
    cli("field", "alter", "User", "full_name", "--max-length", "150")
    cli("index", "add", "User", "email", "--unique")
    cli("generate", "migration")
    sql = (app / "migrations/000002_mach.sql").read_text()
    assert 'RENAME COLUMN "name" TO "full_name"' in sql and "DROP COLUMN" not in sql
    assert json.loads(cli("generate", "migration"))["files"] == []
    cli("field", "remove", "User", "phone", ok=False)
    assert json.loads(cli("field", "remove", "User", "phone", "--dry-run"))["destructive"]
    assert json.loads(cli("inspect", "User", "--sql"))["queries"]["find_by_email"].endswith('WHERE "email" = $1 ORDER BY "id" LIMIT 1')
    original = repo.read_bytes()
    cli("field", "rename", "User", "email", "login")
    assert "find_by_login" in repo.read_text()
    cli("field", "rename", "User", "login", "email")
    assert repo.read_bytes() == original
    assert cli("context", "--compact") == cli("context", "--compact")
    cfg = app / "mach.toml"
    valid = cfg.read_bytes()
    cfg.write_bytes(valid + b"\nunknown_key = 0\n")
    cli("context", ok=False)
    cfg.write_bytes(valid)
    changes = {"src/modules/users/model.rs": {"before":model.read_text(), "after":"broken edit"},
               "src/modules/users/unwritten.rs":{"before":None,"after":"never created"}}
    journal = app / ".mach/journal.json"
    journal.write_text(json.dumps(changes))
    model.write_text("broken edit")
    cli("context")
    assert model.read_text() == changes["src/modules/users/model.rs"]["before"] and not journal.exists()

    # Build rejects invalid derived declarations even without invoking MACH.
    for source in ("find_by_unknown(email: &str)", "find_by_email()", "find_by_email(email: i64)",
                   "find_by_email_contains(email: &str)", "count_by_email(email: &str)"):
        repo.write_bytes(original.replace(b"find_by_email(email: &str)", source.encode()))
        proc = subprocess.run(["cargo","check","--quiet"], cwd=app, capture_output=True)
        assert proc.returncode and b"repository" in proc.stderr, source
    repo.write_bytes(original)
    cli("check", "--agent")
    cli("test")
    if not DB:
        return
    cli("migrate")
    cli("migrate")
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    cfg.write_text(cfg.read_text().replace("[http]", f'[http]\nbind = "127.0.0.1:{port}"'))
    cli("build", "--release")
    token = secrets.token_urlsafe(32)
    grants = "users:create:collection,users:list:collection,users:read:1,users:update:1,users:delete:1"
    server = subprocess.Popen([Path(os.environ["CARGO_TARGET_DIR"]) / "release/demo"], cwd=app,
                              env=dict(os.environ, MACH_TOKEN=token, MACH_GRANTS=grants))
    def request(path, method="GET", data=None, auth=True):
        headers = {"Content-Type":"application/json"}
        if auth:
            headers["Authorization"] = "Bearer " + token
        req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", method=method, headers=headers,
                                     data=json.dumps(data).encode() if data is not None else None)
        try:
            with urllib.request.urlopen(req, timeout=4) as res:
                return res.status, res.read()
        except urllib.error.HTTPError as res:
            return res.code, res.read()
    try:
        for _ in range(200):
            if server.poll() is not None:
                raise RuntimeError("server exited during startup")
            try:
                if request("/ready")[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise RuntimeError("readiness timeout")
        assert request("/users", auth=False)[0] == 401
        assert request("/users/2")[0] == 403
        user = {"email":"ana@example.com","full_name":"Ana"}
        for extra in ({"id":123}, {"phone":"private"}, {"role":"admin"}, {"full_name":"A"}):
            assert request("/users","POST",user | extra)[0] == 422
        status, body = request("/users","POST",user)
        assert status == 201 and json.loads(body) == user | {"id":1}, (status,body)
        assert request("/users","POST",user)[0] == 409
        assert request("/users?limit=101")[0] == 422
        assert json.loads(request("/users?limit=1")[1]) == [user | {"id":1}]
        updated = user | {"full_name":"Ana Silva"}
        assert json.loads(request("/users/1","PUT",updated)[1]) == updated | {"id":1}
        assert json.loads(request("/users/1")[1]) == updated | {"id":1}
        assert request("/users/1","DELETE")[0] == 204
        assert request("/users/1")[0] == 404
    finally:
        server.terminate()
        with contextlib.suppress(subprocess.TimeoutExpired):
            server.wait(5)
        if server.poll() is None:
            server.kill()
            server.wait()
    db_test("live_db")
    cli("field", "remove", "User", "phone", "--allow-destructive")
    cli("generate", "migration", ok=False)
    cli("generate", "migration", "--allow-destructive")
    cli("migrate", ok=False)
    cli("migrate", "--allow-destructive")
    cli("migrate")
    cli("field", "add", "User", "age:i32", "--writable")
    assert json.loads(cli("generate", "migration"))["review"]
    proc = subprocess.run([BIN, "migrate"], cwd=app, capture_output=True, text=True)
    assert proc.returncode and "null values" in proc.stderr, proc.stderr


with tempfile.TemporaryDirectory(prefix="mach-workflow-") as tmp:
    check_app(Path(tmp))
