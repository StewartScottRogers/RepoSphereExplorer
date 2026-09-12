module Test.Main where

import Prelude

import Data.CsvStats (Column(..), mean, summarise)
import Data.Maybe (Maybe(..))
import Effect (Effect)
import Effect.Console (log)

main :: Effect Unit
main = do
  log (show (mean empty))
  log (show (mean threeReadings))
  log (show (summarise threeReadings))

empty :: Column
empty = Column { name: "empty", values: [] }

threeReadings :: Column
threeReadings = Column { name: "n", values: [1.0, 2.0, 3.0] }
