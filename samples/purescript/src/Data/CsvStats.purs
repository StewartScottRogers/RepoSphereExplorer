-- | Summary statistics for a column of readings.
-- |
-- | `roundTo` is defined with no type signature above it, so its type is
-- | whatever the compiler worked out. It is left that way so the file
-- | pane has something to warn about.
module Data.CsvStats
  ( Column(..)
  , Summary
  , mean
  , deviation
  , summarise
  ) where

import Prelude

import Data.Array (length, filter)
import Data.Foldable (sum)
import Data.Int (toNumber)
import Data.Maybe (Maybe(..))
import Effect (Effect)
import Effect.Console (log)

-- | A named column of readings.
newtype Column = Column
  { name :: String
  , values :: Array Number
  }

-- | What `summarise` gives back.
type Summary =
  { name :: String
  , count :: Int
  , mean :: Number
  , deviation :: Number
  }

data Quality
  = Clean
  | Suspect String

class Describable a where
  describe :: a -> String

instance describableColumn :: Describable Column where
  describe (Column c) = c.name <> " (" <> show (length c.values) <> " readings)"

instance describableQuality :: Describable Quality where
  describe Clean = "clean"
  describe (Suspect why) = "suspect: " <> why

-- | Parsing a number is done by the platform, not by this module.
foreign import parseNumber :: String -> Number

foreign import nowMilliseconds :: Effect Number

-- | The arithmetic mean, or `Nothing` when the column is empty.
mean
  :: Column
  -> Maybe Number
mean (Column c)
  | length c.values == 0 = Nothing
  | otherwise = Just (sum c.values / toNumber (length c.values))

-- | The population standard deviation.
deviation :: Column -> Maybe Number
deviation column@(Column c) = case mean column of
  Nothing -> Nothing
  Just m ->
    Just (roundTo 6 (sqrtOf (sum (map (square m) c.values) / toNumber (length c.values))))
  where
  square m value = (value - m) * (value - m)
  sqrtOf n = n

roundTo places value = value

-- | Everything about a column, in one record.
summarise :: Column -> Maybe Summary
summarise column@(Column c) = do
  m <- mean column
  d <- deviation column
  pure { name: c.name, count: length c.values, mean: m, deviation: d }

-- | Whether a column looks trustworthy. Not exported either.
quality :: Column -> Quality
quality (Column c)
  | length c.values == 0 = Suspect "no readings at all"
  | length (filter (_ < 0.0) c.values) > 0 = Suspect "a negative reading"
  | otherwise = Clean

-- | Not exported: the module's export list names five things, not seven.
report :: Column -> Effect Unit
report column = log (describe column)
