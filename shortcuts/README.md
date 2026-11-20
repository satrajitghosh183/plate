# Apple Health, for free: two Shortcuts automations

Apple has no web API for Health, so the iPhone sends the data itself. Two personal
automations in the Shortcuts app do it. Both use the **phone key**, made on the server with
`scripts/plate.sh phone-key` (the app has no screen for it). That key can only send data. It
cannot read anything, and making a new one stops the old one at once.

Replace `API` below with the backend's address (for example `https://plate-api-acbv.onrender.com`)
and `KEY` with the phone key.

## 1. When a strength workout ends

This closes the session in the app and saves it, with no tap.

1. Shortcuts › Automation › **+** › **Apple Watch Workout**.
2. Choose **Ends**. Under workout types, choose your strength types
   (Traditional Strength Training, Functional Strength Training, Core Training).
   Leave runs and rides out, so they never close a session.
3. Choose **Run Immediately** and turn **Notify When Run** off.
4. Add the action **Get Contents of URL**:
   - URL: `API/ingest/workout-ended`
   - Method: **POST**
   - Headers: `x-ingest-key` = `KEY`
   - Request Body: **Text**, `kind=Traditional Strength Training`
5. Optional, so the numbers arrive right away instead of in the evening: before the URL
   action, add **Find Health Samples** › Workouts, sorted by end date, limit 1, and build
   a text body with one more line:

   ```
   kind=Traditional Strength Training
   ```

   then a second **Get Contents of URL** to `API/ingest/health` with the evening format below.
   This only works if Apple Health can be read at that moment, which needs the phone unlocked.

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
3. Add **Find Health Samples** › Workouts, "Start Date is today".
4. Add **Repeat with Each** over the workouts and build one line per workout with **Text**:

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

A zero or missing number never replaces a number already stored for that day. Lifesum can
blank old days when its diary is browsed, so the app keeps what it already had.

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
