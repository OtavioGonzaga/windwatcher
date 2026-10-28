local k = key(ARGV[1])
check_type(k,'hash')
if not fenced(k,ARGV[2],ARGV[3]) then return 0 end
remove_due(k,ARGV[1],'leased')
redis.call('HSET',k,'lease_expires_at',ARGV[4])
redis.call('ZADD',due('leased'),0,ARGV[4] .. ':' .. ARGV[1])
return 1
