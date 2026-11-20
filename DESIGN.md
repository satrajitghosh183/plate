# Design rules

Approved on 2026-11-20. Obey this file before writing any UI.

## Direction

Editorial, on Liquid Glass. One direction, no mixing.

## Colour

- Ground: true black `#000000`.
- Accent: white `#f5f5f7`. There is no second accent. White marks the main action and the live value.
- Text: `#f5f5f7`, secondary `#a1a1a6`, muted `#8a8a8f`. Hairline `rgba(255,255,255,.12)`.
- Error: `#ef6a6b`. Used only for errors.
- The competition plate colours are the only other hues, and each one means a weight: 25 red `#c8443a`, 20 blue `#3566bd`, 15 yellow `#d9a92e`, 10 green `#34935f`, 5 white `#e6e3dc`, small plates steel `#8f8f95`.
- No green, orange, brass or gold accents. No purple or indigo gradients.

## Type

- Newsreader for headlines and hero numbers.
- Geist for text and controls.
- Geist Mono for figures and labels.
- No Inter, Roboto or system fonts.
- Sentence case. Two weights, 400 and 500. Numbers are tabular.

## Material and layout

- Glass is only for the layer that floats: the content pane over the 3D hero, the tab bar capsule, round buttons, sheets and toasts.
- Inside a pane, hierarchy comes from type, space and one hairline. No boxed cards.
- The main button is a white capsule with black text.
- Training screens put a 3D object behind the glass pane: the body model on Today, the loaded barbell while logging.
- No centered three-card hero.
- iPad and desktop (820 px and wider): the 3D hero fills the left, the glass pane is a fixed column on the right, other tabs read in a centred 760 px column, and the tab bar floats under the hero. The app works in any orientation.
- Icons: one plate seen face-on, white on black, as the app icon, favicon (SVG, ICO, PNG) and maskable icon.

## 3D

- Body: smooth, faceless figure in dark glass with the day's muscles lit white. Monochrome. Three renders: `body.jpg` (push, front), `body-pull.jpg` (back view), `body-legs.jpg` (full figure, zoomed to hips and knees). A day picks one when most of its lifts share a group; mixed days use the push render.
- Equipment is drawn in code: loaded bar, dumbbell pair, cable or machine stack with the pin, kettlebell.
- Barbell: drawn in code so it shows the exact plates for the weight and follows kg or lb.

## Motion

- Only movement and fades. Blur never animates.
- Panes and sheets: 520 ms, `cubic-bezier(.32,.72,0,1)`.
- Things that land (plates, tab highlight, toasts): 420 to 460 ms, `cubic-bezier(.3,1.35,.5,1)`.
- Small changes (number rolls, fades): 200 to 320 ms, `cubic-bezier(.2,.8,.2,1)`.
- Buttons press in on touch. iPhone web apps cannot vibrate.
- Respect Reduce Motion: changes become instant.

## Copy

- Real content only. No placeholder text.
- Buttons say what happens: "Start session", "Log set 3".
- Labels and buttons end without a full stop.

## Process

Before any new visual direction: name it, give exact fonts and colours, and wait for a yes. Pilot on one or two screens before changing them all.
