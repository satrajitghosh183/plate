# Apple Health, for free: two Shortcuts automations

Apple has no web API for Health, so the iPhone sends the data itself. Two personal
automations in the Shortcuts app do it. Both use the **phone key**, made on the server with
`scripts/plate.sh phone-key` (the app has no screen for it). That key can only send data. It
cannot read anything. Making a new one leaves the old one working for 24 hours, so you can
update both shortcuts; `scripts/plate.sh phone-key --now` stops the old one at once.

Replace `API` below with the backend's address (for example `https://plate-api-acbv.onrender.com`)
and `KEY` with the phone key.

## 1. When a workout ends

This closes the session in the app and saves it, with no tap. The trigger may include every
workout type. Shortcuts cannot say which workout ended, so the server ignores `kind` and
treats the call as "a workout just ended": it closes an open session at its **last logged
set**, so a run or tennis after lifting never stretches the session. With no session open
it changes nothing.

1. Shortcuts › Automation › **+** › **Apple Watch Workout**.
2. Choose **Ends**. Under workout types, choose your strength types
   (Traditional Strength Training, Functional Strength Training, Core Training).
   Leave runs and rides out, so they never close a session.
3. Choose **Run Immediately** and turn **Notify When Run** off.
4. Add the action **Get Contents of URL**:
   - URL: `API/ingest/workout-ended`
   - Method: **POST**
   - Headers: `x-ingest-key` = `KEY`
   - Request Body: **JSON**, one field `kind` = `Traditional Strength Training` (any value works)
5. Optional, so the numbers arrive right away: add **Run Shortcut** › *Plate · Send today*
   (the evening shortcut below, saved as its own shortcut). A locked phone cannot read Health
   and sends empty values; the server keeps what it had, and the evening run fills it in.

## 2. Every evening

This sends the day's totals (Lifesum writes food to Apple Health) and re-sends the day's
workouts. It catches anything the first automation could not read while the phone was locked.

1. Shortcuts › Automation › **+** › **Time of Day**, 21:30, daily, **Run Immediately**.
2. Add **Find Health Samples** for each of these, "Start Date is today", and **Calculate
   Statistics** › Sum on each:
   - Active Energy
   - Dietary Energy
   - Protein
   - Carbohydrates
   - Total Fat
3. Optional, only if your iPhone offers Workouts in Find Health Samples (some don't; the
   server never needs these lines): **Find Health Samples** › Workouts, "Start Date is today".
4. Then **Repeat with Each** over the workouts and build one line per workout with **Text**:

   ```
   workout=Repeat Item.Type|Repeat Item.Start Date|Repeat Item.End Date|Repeat Item.Active Energy|Repeat Item.Average Heart Rate|Repeat Item.Max Heart Rate
   ```

   Format both dates as ISO 8601 (Format Date › ISO 8601, include time).
5. Add **Text** for the body:

   ```
   date=Current Date (format: yyyy-MM-dd)
   tz=+01:00
   active_energy=Sum of Active Energy
   dietary_energy=Sum of Dietary Energy
   protein=Sum of Protein
   carbs=Sum of Carbohydrates
   fat=Sum of Total Fat
   Repeat Results
   ```

   Use your own offset for `tz`. It is only needed if the dates have no offset.
6. Add **Get Contents of URL**: `API/ingest/health`, **POST**, header `x-ingest-key` = `KEY`,
   Request Body **Text** = the text above.

## What the backend accepts

Plain `key=value` lines (one per line, or `&`-separated) or JSON. Numbers may carry units
and either decimal mark: `1,420`, `1.234,5`, `412 kcal` all work.

| Key | Also accepted as | Meaning |
|---|---|---|
| `date` | `day` | The day the totals belong to, `YYYY-MM-DD` |
| `active_energy` | `energy_burned_kcal`, `burned` | kcal burned |
| `dietary_energy` | `energy_eaten_kcal`, `eaten` | kcal eaten |
| `protein`, `carbs`, `fat` | `protein_g`, `carbs_g`, `fat_g` | grams |
| `workout` | | `kind|start|end|kcal|avg_hr|max_hr` |
| `tz` | `timezone` | Offset for times written without one, e.g. `+01:00` |

The Content-Type does not matter (text, file, octet-stream or none). Many deliveries a day for
the same date are expected: the latest non-zero value of each total wins, so corrections
downwards are kept. An empty, zero or missing value never replaces a stored number: a locked
phone sends empties, and Lifesum can blank old days when its diary is browsed. A body without
a readable `date=YYYY-MM-DD` line is refused with 400 and the reason.

## Check it worked

The backend answers every call with a sentence, for example `Session closed and saved` or
`Saved. Traditional Strength Training: attached`. Shortcuts shows it if you run the
automation by hand. Connect › Sources shows when the last delivery arrived.

## Known limits (test these in Phase 0)

- Nothing can read Apple Health while the phone is locked. The ping still closes the
  session, and the numbers follow with the evening automation.
- Whether "Apple Watch Workout ends" fires with the phone locked, or out of range of the
  watch, is untested.
- Render's free backend may be asleep. The first call can take about a minute; whether
  Shortcuts waits that long is untested. The app's End session tap and the 90-minute rule
  close the session anyway.
