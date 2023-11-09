//! Benchmarking setup for pallet-template
#![cfg(feature = "runtime-benchmarks")]
use super::*;
use scale_info::prelude::vec;

#[allow(unused)]
use crate::Pallet as Template;
use frame_benchmarking::v2::*;
use frame_system::RawOrigin;

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn do_something(s: Linear<0, 100>) {
		let caller: T::AccountId = whitelisted_caller();
		#[extrinsic_call]
		_(RawOrigin::Signed(caller), s);

		assert_eq!(Something::<T>::get(), Some(s));
	}

	impl_benchmark_test_suite!(Template, crate::mock::new_test_ext(), crate::mock::Test,);
}
