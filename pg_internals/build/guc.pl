use strict;
use warnings FATAL => 'all';

use Catalog;
use JSON::PP qw(encode_json);

die "Usage: perl -I POSTGRES_CATALOG_DIR guc.pl INPUT_FILE\n" unless @ARGV == 1;

my $entries = Catalog::ParseData($ARGV[0]);
# Keep the JSON records uniformly string-valued for the Rust generator.
delete $_->{line_number} for @$entries;
print encode_json($entries);
