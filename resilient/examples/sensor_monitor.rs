// sensor_monitor.rs — flagship example.
//
// Demonstrates the full Resilient feature set landed through Phase 1–3
// and the start of Phase 4 (statically-discharged contracts):
//
//   - struct + new {}                                (RES-038)
//   - typed let / fn return type                     (RES-052)
//   - function contracts requires/ensures            (RES-035)
//   - call-site contract folding                     (RES-061)
//   - Result + ? propagation                         (RES-040 / 041)
//   - match expressions                              (RES-039)
//   - arrays + for..in + push                        (RES-032 / 033 / 037)
//   - string + value coercion                        (RES-008)
//   - live { } self-healing block with invariant     (RES-036)
//
// Domain: classify a sequence of sensor readings into "low", "mid",
// "high", or "alert", and total each bucket.

struct Reading {
    int id,
    int value,
}

struct Counts {
    int low,
    int mid,
    int high,
    int alert,
}

// `bucket_of` is statically verifiable for any input >= 0; the
// requires clause proves no negative readings reach the body.
fn bucket_of(int v) -> string
    requires v >= 0
{
    return match true {
        _ => match v < 25 {
            true => "low",
            _ => match v < 75 {
                true => "mid",
                _ => match v < 100 {
                    true => "high",
                    _ => "alert",
                },
            },
        },
    };
}

fn validate(int v) -> Result {
    if v < 0 {
        return Err("negative reading");
    }
    if v > 1000 {
        return Err("overflow reading");
    }
    return Ok(v);
}

// Process one reading: validate, then bucket. ? propagates Err out.
fn process(int v) -> Result {
    let safe = validate(v)?;
    return Ok(bucket_of(safe));
}

fn main() {
    let readings = [
        new Reading { id: 1, value: 10 },
        new Reading { id: 2, value: 50 },
        new Reading { id: 3, value: 90 },
        new Reading { id: 4, value: 200 },
        new Reading { id: 5, value: 0 },
    ];

    let counts = new Counts { low: 0, mid: 0, high: 0, alert: 0 };

    // The live block re-runs the body if any iteration leaves the
    // invariant violated. The invariant says: total never decreases.
    let total = 0;
    live invariant total >= 0 {
        for r in readings {
            let result = process(r.value);
            if is_err(result) {
                println("rejecting reading " + r.id + ": " + unwrap_err(result));
            } else {
                let label = unwrap(result);
                if label == "low" { counts.low = counts.low + 1; }
                if label == "mid" { counts.mid = counts.mid + 1; }
                if label == "high" { counts.high = counts.high + 1; }
                if label == "alert" { counts.alert = counts.alert + 1; }
                total = total + 1;
            }
        }
    }

    println("low:   " + counts.low);
    println("mid:   " + counts.mid);
    println("high:  " + counts.high);
    println("alert: " + counts.alert);
    println("processed: " + total);
}

main();
