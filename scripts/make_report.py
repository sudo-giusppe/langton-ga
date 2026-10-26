#!/usr/bin/env python3
"""Turn the CSV files produced by scripts/run_experiments.sh into the
aggregated series and LaTeX tables used by paper/main.tex.

Only the Python standard library is required.
"""

from __future__ import annotations

import csv
import itertools
import math
import shutil
import subprocess
import re
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RES = ROOT / "results"
PAPER = ROOT / "paper"
DATA = PAPER / "data"
TABLES = PAPER / "tables"
FIGS = PAPER / "figs"

PROFILES = {
    "mover": "0.80,0.05,0.05,0.10",
    "builder": "0.05,0.75,0.10,0.10",
    "weaver": "0.20,0.20,0.50,0.10",
    "balanced": "0.50,0.25,0.15,0.10",
}


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="") as fh:
        return list(csv.DictReader(fh))


def f(row: dict[str, str], key: str, default: float = 0.0) -> float:
    try:
        return float(row[key])
    except (KeyError, TypeError, ValueError):
        return default


def mean(xs: list[float]) -> float:
    return sum(xs) / len(xs) if xs else 0.0


def sd(xs: list[float]) -> float:
    return statistics.stdev(xs) if len(xs) > 1 else 0.0


def fmt(x: float, digits: int = 3) -> str:
    return f"{x:.{digits}f}"


def pm(mean_: float, sd_: float, digits: int = 3) -> str:
    return f"${mean_:.{digits}f} \\pm {sd_:.{digits}f}$"


def thousands(x: float) -> str:
    """Format a count with LaTeX thin spaces as thousands separators."""
    return f"{int(round(x)):,}".replace(",", "\\,")


def write_csv(path: Path, header: list[str], data: list[list[object]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as fh:
        writer = csv.writer(fh)
        writer.writerow(header)
        writer.writerows(data)


def perm_paired(a: list[float], b: list[float]) -> float:
    """Exact two-sided paired permutation test (n is small)."""
    diffs = [x - y for x, y in zip(a, b)]
    observed = abs(mean(diffs))
    count = 0
    total = 2 ** len(diffs)
    for signs in itertools.product((1, -1), repeat=len(diffs)):
        stat = abs(mean([s * d for s, d in zip(signs, diffs)]))
        if stat >= observed - 1e-12:
            count += 1
    return count / total


def load_run(path: Path) -> list[dict[str, str]]:
    data = rows(path)
    data.sort(key=lambda r: int(r["generation"]))
    return data


def aggregate_runs(runs: list[list[dict[str, str]]], key: str = "best") -> dict:
    n = min(len(r) for r in runs)
    per_seed = [[f(r[g], key) for g in range(n)] for r in runs]
    mean_curve = [mean([s[g] for s in per_seed]) for g in range(n)]
    sd_curve = [sd([s[g] for s in per_seed]) for g in range(n)]
    return {"n": n, "per_seed": per_seed, "mean": mean_curve, "sd": sd_curve}


def cluster_runs(cluster_dir: Path, islands: int = 4) -> list[dict[str, str]]:
    """Merge the four island histories into a cluster history.

    At each generation the cluster's best is the best over islands; elapsed
    time is the slowest island (makespan)."""
    histories = []
    for i in range(1, islands + 1):
        path = cluster_dir / f"island{i}.csv"
        if not path.exists():
            continue
        histories.append(load_run(path))
    if not histories:
        return []
    n = min(len(h) for h in histories)
    merged = []
    for g in range(n):
        best = max(f(h[g], "best") for h in histories)
        elapsed = max(int(h[g]["elapsed_ms"]) for h in histories)
        div = mean([f(h[g], "behavioral_diversity") for h in histories])
        merged.append(
            {
                "generation": str(g),
                "best": str(best),
                "behavioral_diversity": str(div),
                "elapsed_ms": str(elapsed),
            }
        )
    return merged


def run_fitness(path: Path) -> float:
    for line in path.read_text().splitlines():
        if line.startswith("fitness="):
            return float(line.split("=", 1)[1])
    return 0.0


def run_meta(path: Path) -> dict[str, float]:
    meta = {}
    for line in path.read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            try:
                meta[k] = float(v)
            except ValueError:
                meta[k] = v
    return meta


def time_to_threshold(run: list[dict[str, str]], threshold: float) -> tuple[int, int]:
    for row in run:
        if f(row, "best") >= threshold:
            return int(row["generation"]), int(row["elapsed_ms"])
    return -1, -1


def main() -> None:
    DATA.mkdir(parents=True, exist_ok=True)
    TABLES.mkdir(parents=True, exist_ok=True)
    FIGS.mkdir(parents=True, exist_ok=True)

    seeds = [1, 2, 3, 4, 5]
    numbers: dict[str, str] = {}

    # ------------------------------------------------------------------
    # Baseline: classic ant
    # ------------------------------------------------------------------
    classic = rows(RES / "baseline" / "classic_metrics.csv")[0]
    trace = rows(RES / "baseline" / "trace.csv")
    trace_pts = []
    step_every = 200
    for row in trace:
        step = int(row["step"])
        if step % step_every == 0 or step == int(trace[-1]["step"]):
            trace_pts.append([step, f(row, "displacement")])
    write_csv(DATA / "classic_trace.csv", ["step", "displacement"], trace_pts)

    numbers["ClassicPeriod"] = str(int(f(classic, "cycle_period")))
    numbers["ClassicDx"] = str(int(f(classic, "drift_x")))
    numbers["ClassicDy"] = str(int(f(classic, "drift_y")))
    numbers["ClassicSpeed"] = fmt(
        math.hypot(f(classic, "drift_x"), f(classic, "drift_y"))
        / max(1.0, f(classic, "cycle_period")),
        4,
    )
    numbers["ClassicMatch"] = fmt(f(classic, "pattern_match"), 4)
    numbers["ClassicNewCells"] = f"{int(f(classic, 'new_cells'))}"
    numbers["ClassicFitness"] = fmt(f(classic, "fitness"))
    numbers["ClassicMobility"] = fmt(f(classic, "mobility"), 4)
    numbers["ClassicCompactness"] = fmt(f(classic, "compactness"), 4)
    numbers["ClassicComplexity"] = fmt(f(classic, "complexity"), 4)

    # ------------------------------------------------------------------
    # Throughput benchmark (optional)
    # ------------------------------------------------------------------
    bench_path = RES / "bench.txt"
    if bench_path.exists():
        bench = {}
        for line in bench_path.read_text().splitlines():
            m = re.match(r"^(iterations|elapsed|evaluations/s|steps/s|cells/s)\s+([0-9.]+)", line)
            if m:
                bench[m.group(1)] = float(m.group(2))
        if "steps/s" in bench:
            numbers["BenchStepsPerSecond"] = f"{bench['steps/s']:.0f}"
        if "steps/s" in bench:
            numbers["BenchMillionsSteps"] = f"{bench['steps/s']/1e6:.2f}"
        if "evaluations/s" in bench:
            numbers["BenchEvalsPerSecond"] = f"{bench['evaluations/s']:.1f}"
        if "cells/s" in bench:
            numbers["BenchMillionsCells"] = f"{bench['cells/s']/1e6:.2f}"
        table = [
            r"\begin{tabular}{lr}",
            r"\toprule",
            r"M\'etrica & Valor \\",
            r"\midrule",
            f"Evaluaciones por segundo & {bench.get('evaluations/s', 0):.1f} \\\\",
            f"Pasos de simulaci\\'on por segundo & {thousands(bench.get('steps/s', 0))} \\\\",
            f"Millones de pasos por segundo & {bench.get('steps/s', 0)/1e6:.2f} \\\\",
            f"Celdas visitadas por segundo & {thousands(bench.get('cells/s', 0))} \\\\",
            r"\bottomrule",
            r"\end{tabular}",
        ]
        (TABLES / "bench.tex").write_text("\n".join(table) + "\n")

    # ------------------------------------------------------------------
    # Single-island convergence
    # ------------------------------------------------------------------
    single = {}
    for profile in ("weaver", "balanced"):
        runs = [load_run(RES / "single" / f"{profile}_{s}.csv") for s in seeds]
        agg = aggregate_runs(runs)
        single[profile] = agg
        header = ["gen"] + [f"seed{s}" for s in seeds] + ["mean", "sd", "bdiv_mean"]
        data = []
        for g in range(agg["n"]):
            divs = [f(r[g], "behavioral_diversity") for r in runs]
            data.append(
                [g]
                + [agg["per_seed"][i][g] for i in range(len(seeds))]
                + [agg["mean"][g], agg["sd"][g], mean(divs)]
            )
        write_csv(DATA / f"single_{profile}.csv", header, data)
        finals = [run_fitness(RES / "single" / f"{profile}_{s}_best.txt") for s in seeds]
        numbers[f"Single{profile.capitalize()}Best"] = pm(mean(finals), sd(finals), 4)
        numbers[f"Single{profile.capitalize()}BestMean"] = fmt(mean(finals))
        numbers[f"Single{profile.capitalize()}BestSd"] = fmt(sd(finals), 4)

    # ------------------------------------------------------------------
    # Distributed comparison: seq / pan / island (migration 5)
    # ------------------------------------------------------------------
    def series_for_mode(mode: str) -> tuple[list[list[dict[str, str]]], list[float], list[float], list[float], list[float]]:
        runs = []
        wall = []
        evals = []
        msgs = []
        byts = []
        if mode in ("seq", "pan"):
            for seed in seeds:
                path = RES / "distributed" / f"{mode}_{seed}.csv"
                runs.append(load_run(path))
                meta = run_meta(RES / "distributed" / f"{mode}_{seed}_best.txt")
                wall.append(float(meta.get("elapsed_ms", 0.0)))
                evals.append(float(meta.get("evaluations", 0.0)))
                msgs.append(0.0)
                byts.append(0.0)
        else:
            for seed in seeds:
                cluster = RES / "distributed" / f"{mode}_{seed}"
                runs.append(cluster_runs(cluster))
                finals = rows(cluster / "finals.csv")
                wall.append(max(int(r["elapsed_ms"]) for r in finals))
                # Cumulative evaluations: sum the four island summaries.
                total_evals = 0.0
                for i in range(1, 5):
                    total_evals += float(
                        run_meta(cluster / f"island{i}_best.txt").get(
                            "evaluations", 0.0
                        )
                    )
                evals.append(total_evals)
                msgs.append(sum(int(r["messages"]) for r in finals))
                byts.append(sum(int(r["bytes"]) for r in finals))
        return runs, wall, evals, msgs, byts

    MODE = {"seq": "Seq", "pan": "Pan", "m5": "Isl"}
    modes = {}
    for mode in ("seq", "pan", "m5"):
        runs, wall, evals, msgs, byts = series_for_mode(mode)
        agg = aggregate_runs(runs)
        finals = [f(r[-1], "best") for r in runs]
        # Late-run behavioural diversity: mean over the last 16 generations.
        # The very last generation is dominated by immigrants injected a few
        # generations earlier, so a single-generation snapshot would conflate a
        # transient with an equilibrium property.
        bdiv = [
            mean([f(r[g], "behavioral_diversity") for g in range(max(0, len(r) - 16), len(r))])
            for r in runs
        ]
        auc = [mean([f(r[g], "best") for g in range(len(r))]) for r in runs]
        t70 = [time_to_threshold(r, 0.70) for r in runs]
        modes[mode] = {
            "runs": runs,
            "agg": agg,
            "finals": finals,
            "bdiv": bdiv,
            "wall": wall,
            "evals": evals,
            "msgs": msgs,
            "bytes": byts,
            "auc": auc,
            "t70_gen": [x[0] for x in t70],
            "t70_ms": [x[1] for x in t70],
        }
        header = ["gen", "mean", "sd", "bdiv_mean", "elapsed_mean"] + [
            f"seed{s}" for s in seeds
        ]
        data = []
        for g in range(agg["n"]):
            elapsed = mean([float(r[g]["elapsed_ms"]) for r in runs])
            bdiv_g = mean([f(r[g], "behavioral_diversity") for r in runs])
            data.append(
                [g, agg["mean"][g], agg["sd"][g], bdiv_g, elapsed]
                + [agg["per_seed"][i][g] for i in range(len(seeds))]
            )
        write_csv(DATA / f"dist_{mode}.csv", header, data)
        numbers[f"Wall{MODE[mode]}"] = fmt(mean(wall) / 1000.0, 2)
        numbers[f"Wall{MODE[mode]}Sd"] = fmt(sd(wall) / 1000.0, 2)
        numbers[f"Evals{MODE[mode]}"] = f"{mean(evals):.0f}"
        numbers[f"Final{MODE[mode]}"] = pm(mean(finals), sd(finals))
        numbers[f"Final{MODE[mode]}Mean"] = fmt(mean(finals))
        numbers[f"Auc{MODE[mode]}"] = fmt(mean(auc))
        numbers[f"Bdiv{MODE[mode]}"] = fmt(mean(bdiv), 3)
        numbers[f"Msgs{MODE[mode]}"] = f"{mean(msgs):.0f}"
        numbers[f"Bytes{MODE[mode]}"] = f"{mean(byts)/1024.0:.1f}"
        valid_t = [x for x in modes[mode]["t70_ms"] if x >= 0]
        numbers[f"TSeventy{MODE[mode]}"] = (
            fmt(mean(valid_t) / 1000.0, 2) if valid_t else "--"
        )

    speedup = mean(modes["seq"]["wall"]) / mean(modes["m5"]["wall"])
    speedup_pan = mean(modes["seq"]["wall"]) / mean(modes["pan"]["wall"])
    numbers["SpeedupIsl"] = fmt(speedup, 2)
    numbers["SpeedupPan"] = fmt(speedup_pan, 2)
    numbers["EfficiencyIsl"] = fmt(100.0 * speedup / 4.0, 1)
    numbers["PvalueIslPan"] = fmt(
        perm_paired(modes["m5"]["finals"], modes["pan"]["finals"]), 4
    )
    numbers["PvalueIslSeq"] = fmt(
        perm_paired(modes["m5"]["finals"], modes["seq"]["finals"]), 4
    )
    numbers["OverheadBytesPerEval"] = fmt(
        mean(modes["m5"]["bytes"]) / mean(modes["m5"]["evals"]), 4
    )

    # LaTeX table: distributed comparison
    r = modes
    table = [
        r"\begin{tabular}{lrrrrrr}",
        r"\toprule",
        r"Modo & Tiempo (s) & Acel. & Evaluaciones & Mejor final & AUC & Div. conductual \\",
        r"\midrule",
        f"Secuencial & {fmt(mean(r['seq']['wall'])/1000,2)} & 1.00 & {mean(r['seq']['evals']):.0f} & {pm(mean(r['seq']['finals']), sd(r['seq']['finals']))} & {fmt(mean(r['seq']['auc']))} & {fmt(mean(r['seq']['bdiv']))} \\\\",
        f"Panm\\'ictico & {fmt(mean(r['pan']['wall'])/1000,2)} & {fmt(speedup_pan,2)} & {mean(r['pan']['evals']):.0f} & {pm(mean(r['pan']['finals']), sd(r['pan']['finals']))} & {fmt(mean(r['pan']['auc']))} & {fmt(mean(r['pan']['bdiv']))} \\\\",
        f"Islas (m=5) & {fmt(mean(r['m5']['wall'])/1000,2)} & {fmt(speedup,2)} & {mean(r['m5']['evals']):.0f} & {pm(mean(r['m5']['finals']), sd(r['m5']['finals']))} & {fmt(mean(r['m5']['auc']))} & {fmt(mean(r['m5']['bdiv']))} \\\\",
        r"\bottomrule",
        r"\end{tabular}",
    ]
    (TABLES / "distributed.tex").write_text("\n".join(table) + "\n")

    # ------------------------------------------------------------------
    # Migration interval ablation
    # ------------------------------------------------------------------
    MIGNAME = {0: "None", 5: "Five", 20: "Twenty"}
    migration = {}
    for mode, interval in (("m0", 0), ("m5", 5), ("m20", 20)):
        runs, wall, evals, msgs, byts = series_for_mode(mode)
        finals = [f(run[-1], "best") for run in runs]
        bdiv = [
            mean([f(run[g], "behavioral_diversity") for g in range(max(0, len(run) - 16), len(run))])
            for run in runs
        ]
        migration[interval] = {
            "finals": finals,
            "bdiv": bdiv,
            "wall": wall,
            "msgs": msgs,
            "bytes": byts,
            "evals": evals,
            "auc": [mean([f(run[g], "best") for g in range(len(run))]) for run in runs],
        }
        numbers[f"Mig{MIGNAME[interval]}Best"] = pm(mean(finals), sd(finals))
        numbers[f"Mig{MIGNAME[interval]}Bdiv"] = fmt(mean(bdiv), 3)
        numbers[f"Mig{MIGNAME[interval]}Auc"] = fmt(mean(migration[interval]["auc"]))
        numbers[f"Mig{MIGNAME[interval]}Time"] = fmt(mean(wall) / 1000, 2)
        numbers[f"Mig{MIGNAME[interval]}Kb"] = fmt(mean(byts) / 1024.0, 1)
        numbers[f"Mig{MIGNAME[interval]}Msgs"] = f"{mean(msgs):.0f}"

    write_csv(
        DATA / "migration.csv",
        ["interval", "best_mean", "best_sd", "bdiv_mean", "bdiv_sd", "time_mean", "time_sd", "kb_mean", "msgs_mean"],
        [
            [
                interval,
                mean(v["finals"]),
                sd(v["finals"]),
                mean(v["bdiv"]),
                sd(v["bdiv"]),
                mean(v["wall"]) / 1000,
                sd(v["wall"]) / 1000,
                mean(v["bytes"]) / 1024,
                mean(v["msgs"]),
            ]
            for interval, v in sorted(migration.items())
        ],
    )
    p_m0_m5 = perm_paired(migration[0]["finals"], migration[5]["finals"])
    p_m5_m20 = perm_paired(migration[5]["finals"], migration[20]["finals"])
    numbers["PvalueMigNoneFive"] = fmt(p_m0_m5, 4)
    numbers["PvalueMigFiveTwenty"] = fmt(p_m5_m20, 4)
    p_div_0_5 = perm_paired(migration[0]["bdiv"], migration[5]["bdiv"])
    numbers["PvalueDivMigNoneFive"] = fmt(p_div_0_5, 4)

    table = [
        r"\begin{tabular}{rrrrrr}",
        r"\toprule",
        r"Intervalo & Mejor final & AUC & Div. conductual & Tiempo (s) & Control (KB) \\",
        r"\midrule",
    ]
    for interval, v in sorted(migration.items()):
        table.append(
            f"{interval} & {pm(mean(v['finals']), sd(v['finals']))} & {fmt(mean(v['auc']))} & {fmt(mean(v['bdiv']))} & {fmt(mean(v['wall'])/1000,2)} & {fmt(mean(v['bytes'])/1024,1)} \\\\"
        )
    table += [r"\bottomrule", r"\end{tabular}"]
    (TABLES / "migration.tex").write_text("\n".join(table) + "\n")

    # ------------------------------------------------------------------
    # Portfolio (heterogeneous islands, no migration)
    # ------------------------------------------------------------------
    portfolio_rows = []
    for i, profile in enumerate(("mover", "builder", "weaver", "balanced"), start=1):
        best = run_meta(RES / "portfolio" / f"island{i}_best.txt")
        analyze_path = RES / "gallery" / f"portfolio_{i}.csv"
        a = rows(analyze_path)[0] if analyze_path.exists() else {}
        period = int(f(a, "cycle_period")) if a else 0
        speed = (
            math.hypot(f(a, "drift_x"), f(a, "drift_y")) / period if period else 0.0
        )
        portfolio_rows.append(
            {
                "profile": profile,
                "hex": best.get("hex", ""),
                "fitness": float(best.get("fitness", 0.0)),
                "fitness_weaver": f(a, "fitness") if a else 0.0,
                "mobility": f(a, "mobility") if a else float(best.get("mobility", 0)),
                "compactness": f(a, "compactness") if a else float(best.get("compactness", 0)),
                "complexity": f(a, "complexity") if a else float(best.get("complexity", 0)),
                "neighborhoods": int(f(a, "neighborhoods")) if a else 0,
                "period": period,
                "drift_x": int(f(a, "drift_x")) if a else 0,
                "drift_y": int(f(a, "drift_y")) if a else 0,
                "speed": speed,
            }
        )
        numbers[f"Portfolio{profile.capitalize()}Fit"] = fmt(float(best.get("fitness", 0.0)))
        numbers[f"Portfolio{profile.capitalize()}Speed"] = fmt(speed if speed else float(best.get("mobility", 0.0)), 3)

    table = [
        r"\begin{tabular}{lrrrrrrr}",
        r"\toprule",
        r"Perfil & Aptitud & Apt.\ weaver & Movilidad & Densidad & Compl. & Vec. & Per.\ / deriva \\",
        r"\midrule",
    ]
    for row in portfolio_rows:
        cycle = (
            f"{row['period']} / ({row['drift_x']},{row['drift_y']})"
            if row["period"]
            else "---"
        )
        table.append(
            f"{row['profile']} & {fmt(row['fitness'])} & {fmt(row['fitness_weaver'])} & {fmt(row['mobility'],4)} & {fmt(row['compactness'])} & {fmt(row['complexity'])} & {row['neighborhoods']} & {cycle} \\\\"
        )
    table += [r"\bottomrule", r"\end{tabular}"]
    (TABLES / "portfolio.tex").write_text("\n".join(table) + "\n")

    # ------------------------------------------------------------------
    # Classic baseline table + generic numbers
    # ------------------------------------------------------------------
    table = [
        r"\begin{tabular}{lr}",
        r"\toprule",
        r"M\'etrica & Valor \\",
        r"\midrule",
        f"Periodo de la autopista & {int(f(classic,'cycle_period'))} pasos \\\\",
        f"Deriva por periodo & ({int(f(classic,'drift_x'))}, {int(f(classic,'drift_y'))}) \\\\",
        f"Velocidad de traslaci\\'on & {numbers['ClassicSpeed']} celdas/paso \\\\",
        f"Coincidencia del patr\\'on & {fmt(f(classic,'pattern_match'),4)} \\\\",
        f"Celdas visitadas & {int(f(classic,'new_cells'))} \\\\",
        f"Revisitas & {int(f(classic,'revisits'))} \\\\",
        f"Bounding box & {int(f(classic,'bbox_width'))}$\\times${int(f(classic,'bbox_height'))} \\\\",
        f"Movilidad & {fmt(f(classic,'mobility'),4)} \\\\",
        f"Densidad & {fmt(f(classic,'compactness'),4)} \\\\",
        f"Complejidad & {fmt(f(classic,'complexity'),4)} \\\\",
        f"Vecindades 3$\\times$3 & {int(f(classic,'neighborhoods'))} \\\\",
        f"Aptitud (perfil balanced) & {fmt(f(classic,'fitness'))} \\\\",
        r"\bottomrule",
        r"\end{tabular}",
    ]
    (TABLES / "classic.tex").write_text("\n".join(table) + "\n")

    # ------------------------------------------------------------------
    # Macro file for inline numbers
    # ------------------------------------------------------------------
    lines = ["% Generated by scripts/make_report.py -- do not edit."]
    for key, value in sorted(numbers.items()):
        lines.append(f"\\newcommand{{\\{key}}}{{{value}}}")
    (TABLES / "numbers.tex").write_text("\n".join(lines) + "\n")

    # Copy the best figures into paper/figs with stable names.
    copies = {
        "baseline/classic_12k.png": "classic_12k.png",
        "baseline/classic_120k.png": "classic_120k.png",
        "gallery/weaver.png": "weaver.png",
        "gallery/weaver_60k.png": "weaver_60k.png",
        "gallery/conveyor.png": "conveyor.png",
        "gallery/mover.png": "mover.png",
        "gallery/builder.png": "builder.png",
    }
    for profile in ("mover", "builder", "weaver", "balanced"):
        source = RES / "gallery" / f"traj_{profile}.csv"
        if source.exists():
            shutil.copy2(source, DATA / f"traj_{profile}.csv")

    for src, dst in copies.items():
        source = RES / src
        if source.exists():
            shutil.copy2(source, FIGS / dst)
            # The renders use 3-4 flat colours; an indexed PNG is pixel-identical
            # and roughly 300x smaller, which keeps the PDF light.  This step is
            # optional: without ImageMagick the true-colour copy is used.
            tool = shutil.which("magick") or shutil.which("convert")
            if tool:
                tmp = FIGS / (dst + ".tmp.png")
                try:
                    subprocess.run(
                        [tool, str(FIGS / dst), "-strip", "-colors", "8",
                         "-define", "png:compression-level=9", "PNG8:" + str(tmp)],
                        check=True, capture_output=True,
                    )
                    tmp.replace(FIGS / dst)
                except Exception:
                    tmp.unlink(missing_ok=True)

    print("wrote paper/data, paper/tables and paper/figs")
    print(f"  single weaver best: {numbers['SingleWeaverBest']}")
    print(f"  seq/pan/isl wall: {numbers['WallSeq']} / {numbers['WallPan']} / {numbers['WallIsl']} s")
    print(f"  speedup isl: {numbers['SpeedupIsl']}x (eff {numbers['EfficiencyIsl']}%)")
    print(f"  p(isl vs pan): {numbers['PvalueIslPan']}")
    print(f"  migration p(0 vs 5): {numbers['PvalueMigNoneFive']}")


if __name__ == "__main__":
    main()
