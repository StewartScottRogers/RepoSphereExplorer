# MilestoneEscrow

An escrow that releases funds a milestone at a time, with an owner who can
release and a payer who can be refunded.

## Using it

```solidity
MilestoneEscrow escrow = new MilestoneEscrow();

escrow.deposit{value: 1 ether}();     // as the payer
escrow.release(milestoneId);          // as the owner
escrow.refund();                      // as the payer, if it never shipped
```

## Notes

- `deposit` reverts on zero. A deposit of nothing emits an event that says
  something happened when nothing did.
- Amounts are stored as `uint96` through a checked conversion, not a cast.
  A silent truncation in an escrow is somebody's money.
- Fuzz runs are set high in the `ci` profile and lower by default, so a
  local run stays quick and the pipeline still tries hard.

## Building

```bash
forge build
forge test -vvv
forge fmt --check
```

---

**This is a fixture.** It lives in `samples/solidity/` so the application has a
Solidity project to open, not just a Solidity file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
