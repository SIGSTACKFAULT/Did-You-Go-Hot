# Did You Go Hot? - Wormhole Rolling Calculator

**Did You Go Hot?** is a fast, specialized Rust application designed to calculate optimal wormhole rolling strategies for EVE Online.

## Features

* **Visual GUI:** An easy-to-use interface to configure your roll parameters and to walk you through the rolling steps.
* **Optimal Path Generation:** Calculates the best possible series of jumps (In/Out, Hot/Cold) to safely close a wormhole.
* **Customizable Priorities:** Choose what matters most for your situation:
  * **RO Probability (Roll Out Probability):** Minimize the chance of a ship getting stuck on the wrong side.
  * **Avg Num Passes:** Minimize the average time/number of jumps required to collapse the hole.
  * **Max Out:** Minimize the maximum number of ships outside the wormhole at any given time.
* **Advanced State Management:** Handles all wormhole mass states:
  * `Full`
  * `Shrink`
  * `Crit`
* **Custom Ships & Mass Limitations:** Supports inputting custom hot/cold ship masses and specifying known maximum or minimum already passed mass.
* **Polarization Guide:** Configure limits on ship polarizations to ensure the generated plans are practical for your available pilots.

## Prerequisites

To build and run this project, you will need to use a **Nightly** Rust toolchain because the project relies on unstable features:
```rust
#![feature(portable_simd)]
#![feature(int_roundings)]
```

## Building and Running

### Pre-compiled Binaries
If you prefer not to build the application yourself, you can download pre-compiled executables for your operating system from the **[Releases](https://github.com/your-username/your-repo/releases)** page.

### Building from Source
If you want to compile the project locally:

1. Clone the repository to your local machine.
2. Navigate to the project directory.
3. Ensure you have the nightly toolchain active for this directory:
   ```bash
   rustup override set nightly
   ```
4. Build and run the app via Cargo:
    ```bash
    cargo run --release
    ```

Note: Due to the intensive nature of the pathfinding algorithm, it is highly recommended to run the project in --release mode for optimal calculation speed. Running in dev/debug mode will be significantly slower.
Usage

## Usage

1. Select Hole Size: Input your target wormhole's mass (e.g., 3.3B, 2B, 1B).

2. Set Initial State: Specify whether the hole is currently Full, Shrink, or Crit.

3. Configure Ships: Enable the rolling ships you have available. You can customize the exact Hot and Cold mass profiles for each ship, as well as how many of each you have available.

4. Define Priorities: Adjust the qualities ranking to tell the algorithm whether it should prioritize safety (RO Probability), speed (Average Passes), or minimizing risk/number of toons on the other side (Max Out).

5. Calculate: Start the calculator to generate your roll chart.

6. Select the roll plan that best fits your needs.

7. Follow the Chart: Use the interactive walkthrough panel to log your jumps as you do them in EVE. The app will tell you the optimal next step based on whether the wormhole state changes dynamically.

## Planned Features

1. Roll saver. Did someone jump cold when they were supposed to jump hot? The calculator should be able to take all the jumps you have already done as input and attempt to minimize the rollout probability. The sequence of jumps provides vital information on the potential max mass range which can help to minimize a rollout.

2. Better rolling graph simplification. (e.g. If its the same actions if it is shrink or crit combine it into one path.)

3. Colored visual graph to make it more clear.

4. Better probability calculation.

## Limitations

Probabilities are based on assuming that the maximum hole size is uniform random and that the current mass is uniform random. Both of these assumptions could be incorrect. Additionally, the current probability estimations are not 100% accurate. This influences both the average number of passes and the RO probability. However, they are geneerally in the correct ballpark.

Very complex rolls with many allowed polorizations and many rollers can be expensive and memory intensive to calculate.
