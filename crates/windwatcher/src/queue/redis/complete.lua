local id = ARGV[1]
local k = key(id)
check_type(k,'hash')
if not fenced(k,ARGV[2],ARGV[3]) then return 0 end
member(k,id)
remove_due(k,id,'leased')
move(k,id,'leased','completed')
clear_lease(k)
redis.call('HDEL',k,'last_error_code','last_error_message')
redis.call('HSET',k,'completed_at',ARGV[3])
redis.call('ZADD',completed,0,ARGV[3] .. ':' .. id)
return 1
